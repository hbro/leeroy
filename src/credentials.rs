//! Getting the credentials a connection needs, for the executor in
//! `main.rs`: the basic-auth password from its store, OpenID Connect tokens
//! from the store (refreshed) or a login. Stores what it obtains.

use std::{
    path::Path,
    time::{Duration, SystemTime},
};

use crate::{
    auth::{
        self, AuthConfig, Credentials, LoginFlow, LoginPrompt, OidcConfig, PasswordStore, Resolved,
        Secret, SecretStore, TokenSet,
    },
    jenkins::{self, ConnectionConfig},
    oidc, secrets,
};

/// A browser login nobody finishes is given up after this long.
const BROWSER_LOGIN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// What `config.auth` needs before connecting. `password`: known without a
/// store (env var, config file); `tokens`: this session's.
pub async fn resolve(
    config: &ConnectionConfig,
    password: Option<Secret>,
    tokens: Option<TokenSet>,
    tokens_path: &Path,
) -> Result<Resolved, String> {
    match &config.auth {
        AuthConfig::Headers | AuthConfig::Anonymous => Ok(Resolved::Credentials(Credentials::None)),
        AuthConfig::Basic(basic) => {
            if let Some(missing) = basic.missing(password.is_some()) {
                return Err(missing.message().into());
            }
            let username = basic.username.clone().unwrap_or_default();
            let password = match (password, basic.store, &basic.command) {
                (Some(password), _, _) => password,
                (None, PasswordStore::Command, Some(command)) => {
                    secrets::password_from_command(command, secrets::COMMAND_TIMEOUT).await?
                }
                // `missing` covers the file store and a missing command.
                _ => {
                    let account = auth::password_account(&config.url, &username);
                    secrets::keyring_get(&account)
                        .await?
                        .ok_or("no password in the OS keyring yet: set it in the settings (0)")?
                }
            };
            Ok(Resolved::Credentials(Credentials::Basic {
                username,
                password,
            }))
        }
        AuthConfig::Oidc(oidc) => resolve_tokens(config, oidc, tokens, tokens_path).await,
    }
}

async fn resolve_tokens(
    config: &ConnectionConfig,
    oidc: &OidcConfig,
    tokens: Option<TokenSet>,
    tokens_path: &Path,
) -> Result<Resolved, String> {
    if let Some(tokens) = &tokens
        && !tokens.refresh_due(SystemTime::now())
    {
        return Ok(Resolved::Tokens(tokens.clone()));
    }
    let refresh = match tokens.and_then(|t| t.refresh) {
        Some(refresh) => Some(refresh),
        None => load_refresh(oidc, tokens_path).await?,
    };
    let Some(refresh) = refresh else {
        return Ok(Resolved::LoginNeeded);
    };
    let provider = Provider::of(config).await?;
    match provider.refresh(&refresh).await {
        Ok(tokens) => {
            keep_refresh_token(oidc, &tokens, Some(&refresh), tokens_path).await?;
            Ok(Resolved::Tokens(tokens))
        }
        Err(err) => {
            // E.g. the SSO session ended: log in again.
            tracing::info!(%err, "stored login no longer works");
            Ok(Resolved::LoginNeeded)
        }
    }
}

/// Log in with `flow`: `started` gets what to show, the result is the tokens
/// (already stored).
pub async fn login(
    config: &ConnectionConfig,
    flow: LoginFlow,
    tokens_path: &Path,
    started: impl FnOnce(LoginPrompt),
) -> Result<TokenSet, String> {
    let Provider {
        oidc,
        client_id,
        client,
        endpoints,
    } = Provider::of(config).await?;
    let tokens = match flow {
        LoginFlow::Device => {
            let device = oidc::start_device(&client, &endpoints, client_id).await?;
            started(device.prompt.clone());
            oidc::poll_device(&client, &endpoints, client_id, &device).await?
        }
        LoginFlow::Browser => {
            let browser = oidc::start_browser(&endpoints, client_id).await?;
            started(browser.prompt.clone());
            tokio::time::timeout(
                BROWSER_LOGIN_TIMEOUT,
                oidc::finish_browser(&client, &endpoints, client_id, browser),
            )
            .await
            .map_err(|_| "the browser login took too long: press r to retry".to_owned())??
        }
    };
    keep_refresh_token(oidc, &tokens, None, tokens_path).await?;
    Ok(tokens)
}

/// A new access token from `refresh` (stored if it changed).
pub async fn refresh(
    config: &ConnectionConfig,
    refresh: &Secret,
    tokens_path: &Path,
) -> Result<TokenSet, String> {
    let provider = Provider::of(config).await?;
    let tokens = provider.refresh(refresh).await?;
    keep_refresh_token(provider.oidc, &tokens, Some(refresh), tokens_path).await?;
    Ok(tokens)
}

/// The connection's OpenID Connect provider, ready for requests.
struct Provider<'a> {
    oidc: &'a OidcConfig,
    client_id: &'a str,
    client: reqwest::Client,
    endpoints: oidc::Endpoints,
}

impl<'a> Provider<'a> {
    /// Client (with the connection's proxy and TLS settings) and endpoints.
    async fn of(config: &'a ConnectionConfig) -> Result<Provider<'a>, String> {
        let AuthConfig::Oidc(oidc) = &config.auth else {
            return Err("not an OpenID Connect connection".into());
        };
        let (issuer, client_id) = oidc.issuer_and_client()?;
        let client = jenkins::client(config)?;
        let endpoints = oidc::discover(&client, issuer).await?;
        Ok(Provider {
            oidc,
            client_id,
            client,
            endpoints,
        })
    }

    async fn refresh(&self, refresh: &Secret) -> Result<TokenSet, String> {
        oidc::refresh(&self.client, &self.endpoints, self.client_id, refresh).await
    }
}

/// Write (`None`: delete) a secret.
pub async fn store(
    store: SecretStore,
    account: &str,
    secret: Option<&Secret>,
    tokens_path: &Path,
) -> Result<(), String> {
    match (store, secret) {
        (SecretStore::Keyring, Some(secret)) => secrets::keyring_set(account, secret).await,
        (SecretStore::Keyring, None) => secrets::keyring_delete(account).await,
        (SecretStore::TokensFile, secret) => {
            secrets::tokens_file_set(tokens_path, account, secret).await
        }
    }
}

/// The stored refresh token, if any.
async fn load_refresh(oidc: &OidcConfig, tokens_path: &Path) -> Result<Option<Secret>, String> {
    let account = oidc.account()?;
    match oidc.store.secret_store() {
        Some(SecretStore::Keyring) => secrets::keyring_get(&account).await,
        Some(SecretStore::TokensFile) => secrets::tokens_file_get(tokens_path, &account).await,
        None => Ok(None),
    }
}

/// Keep the refresh token for the next run (only the refresh token: access
/// tokens are short-lived, and Windows' keyring holds at most 2560 bytes).
async fn keep_refresh_token(
    oidc: &OidcConfig,
    tokens: &TokenSet,
    previous: Option<&Secret>,
    tokens_path: &Path,
) -> Result<(), String> {
    let Some(refresh) = &tokens.refresh else {
        return Ok(());
    };
    if Some(refresh) == previous {
        return Ok(());
    }
    let Some(store_in) = oidc.store.secret_store() else {
        return Ok(());
    };
    store(store_in, &oidc.account()?, Some(refresh), tokens_path)
        .await
        .map_err(|err| format!("logged in, but couldn't keep the login: {err}"))
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;
    use crate::auth::{BasicConfig, LoginFlow, TokenStore};

    fn config(auth: AuthConfig) -> ConnectionConfig {
        ConnectionConfig {
            url: "https://ci.example.com".into(),
            headers: Vec::new(),
            proxy: None,
            skip_tls_verify: false,
            auth,
            credentials: Credentials::None,
        }
    }

    fn oidc(issuer: &str, store: TokenStore) -> OidcConfig {
        OidcConfig {
            issuer: Some(issuer.into()),
            client_id: Some("leeroy".into()),
            flow: LoginFlow::Device,
            store,
        }
    }

    #[tokio::test]
    async fn known_password_wins_over_the_store() {
        let config = config(AuthConfig::Basic(BasicConfig {
            username: Some("me".into()),
            store: PasswordStore::Keyring,
            command: None,
        }));
        let dir = tempfile::tempdir().unwrap();
        let resolved = resolve(&config, Some(Secret::new("env-pw")), None, dir.path())
            .await
            .unwrap();
        assert_eq!(
            resolved,
            Resolved::Credentials(Credentials::Basic {
                username: "me".into(),
                password: Secret::new("env-pw")
            })
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn password_from_a_command() {
        let config = config(AuthConfig::Basic(BasicConfig {
            username: Some("me".into()),
            store: PasswordStore::Command,
            command: Some("echo from-cmd".into()),
        }));
        let dir = tempfile::tempdir().unwrap();
        let resolved = resolve(&config, None, None, dir.path()).await.unwrap();
        assert!(matches!(
            resolved,
            Resolved::Credentials(Credentials::Basic { password, .. })
                if password == Secret::new("from-cmd")
        ));
    }

    async fn provider() -> MockServer {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path("/realm/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token_endpoint": format!("{base}/token"),
            })))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn stored_login_is_refreshed_and_rotation_kept() {
        let server = provider().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=stored"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "fresh", "refresh_token": "rotated", "expires_in": 300
            })))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let tokens_path = secrets::tokens_path(dir.path());
        let oidc = oidc(&format!("{}/realm", server.uri()), TokenStore::File);
        let account = auth::token_account(oidc.issuer.as_deref().unwrap(), "leeroy");
        secrets::tokens_file_set(&tokens_path, &account, Some(&Secret::new("stored")))
            .await
            .unwrap();
        let config = config(AuthConfig::Oidc(oidc));
        let resolved = resolve(&config, None, None, &tokens_path).await.unwrap();
        assert!(matches!(&resolved, Resolved::Tokens(t) if t.access == Secret::new("fresh")));
        assert_eq!(
            secrets::tokens_file_get(&tokens_path, &account).await,
            Ok(Some(Secret::new("rotated")))
        );
    }

    #[tokio::test]
    async fn no_or_rejected_login_means_logging_in() {
        let server = provider().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({ "error": "invalid_grant" })),
            )
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let tokens_path = secrets::tokens_path(dir.path());
        let oidc = oidc(&format!("{}/realm", server.uri()), TokenStore::File);
        let config = config(AuthConfig::Oidc(oidc.clone()));
        assert_eq!(
            resolve(&config, None, None, &tokens_path).await,
            Ok(Resolved::LoginNeeded)
        );
        let account = auth::token_account(oidc.issuer.as_deref().unwrap(), "leeroy");
        secrets::tokens_file_set(&tokens_path, &account, Some(&Secret::new("stale")))
            .await
            .unwrap();
        assert_eq!(
            resolve(&config, None, None, &tokens_path).await,
            Ok(Resolved::LoginNeeded)
        );
    }

    #[tokio::test]
    async fn this_sessions_valid_token_is_reused_without_asking() {
        let oidc = oidc("https://sso.invalid/realm", TokenStore::Memory);
        let tokens = TokenSet {
            access: Secret::new("still-good"),
            refresh: None,
            expires_at: Some(SystemTime::now() + Duration::from_secs(600)),
            refresh_at: Some(SystemTime::now() + Duration::from_secs(570)),
        };
        let dir = tempfile::tempdir().unwrap();
        let resolved = resolve(
            &config(AuthConfig::Oidc(oidc)),
            None,
            Some(tokens.clone()),
            dir.path(),
        )
        .await;
        assert_eq!(resolved, Ok(Resolved::Tokens(tokens)));
    }
}
