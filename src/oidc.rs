//! OpenID Connect (Keycloak) login: discovery, the device code flow, the
//! browser flow (authorization code + PKCE, redirect to 127.0.0.1) and token
//! refresh. IO; the executor in `main.rs` drives it from spawned tasks.
//! Requests use the same proxy and TLS settings as Jenkins.

use std::time::{Duration, SystemTime};

use serde::Deserialize;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::{
    auth::{LoginFlow, LoginPrompt, Pkce, Secret, TokenSet, random_bytes},
    config::redact_url,
    jenkins,
};

/// Asked for with every login: `openid` makes Keycloak issue an ID token
/// and a refresh token tied to the SSO session.
const SCOPE: &str = "openid";

/// The provider's endpoints, from `/.well-known/openid-configuration`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Endpoints {
    pub token_endpoint: String,
    pub authorization_endpoint: Option<String>,
    pub device_authorization_endpoint: Option<String>,
}

/// Look up the endpoints of `issuer`.
pub async fn discover(client: &reqwest::Client, issuer: &str) -> Result<Endpoints, String> {
    let url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|err| format!("identity provider: {}", jenkins::describe(err)))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "identity provider: no OpenID configuration at {} (HTTP {status}); is the \
             issuer URL right? For Keycloak: https://<host>/realms/<realm>",
            redact_url(&url)
        ));
    }
    response
        .json()
        .await
        .map_err(|_| "identity provider: unexpected OpenID configuration".into())
}

/// A token endpoint answer (success or error, RFC 6749 §5).
#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    error: Option<String>,
    error_description: Option<String>,
}

impl TokenResponse {
    fn tokens(self, now: SystemTime) -> Result<TokenSet, String> {
        let access = self
            .access_token
            .ok_or("identity provider: no access token in its answer")?;
        Ok(TokenSet::issued(
            Secret::new(access),
            self.refresh_token.map(Secret::new),
            now,
            self.expires_in.map(Duration::from_secs),
        ))
    }

    fn error(&self) -> String {
        match (&self.error, &self.error_description) {
            (Some(code), Some(description)) => format!("{description} ({code})"),
            (Some(code), None) => code.clone(),
            _ => "unexpected answer".into(),
        }
    }
}

async fn token_request(
    client: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<TokenResponse, String> {
    let (status, bytes) = post_form(client, endpoint, form).await?;
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("identity provider: unexpected answer (HTTP {status})"))
}

/// POST `form` (URL-encoded) to the provider: the answer's status and body.
async fn post_form(
    client: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<(reqwest::StatusCode, Vec<u8>), String> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form)
        .finish();
    let response = client
        .post(endpoint)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .await
        .map_err(|err| format!("identity provider: {}", jenkins::describe(err)))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("identity provider: {}", jenkins::describe(err)))?;
    Ok((status, bytes.to_vec()))
}

/// A started device code login: show `prompt`, then [`poll_device`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLogin {
    pub prompt: LoginPrompt,
    device_code: Secret,
    interval: Duration,
}

#[derive(Deserialize)]
struct DeviceResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: Option<u64>,
    interval: Option<u64>,
}

/// Ask for a device code (RFC 8628).
pub async fn start_device(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    client_id: &str,
) -> Result<DeviceLogin, String> {
    let endpoint = endpoints.device_authorization_endpoint.as_deref().ok_or(
        "the identity provider doesn't offer device login: use the browser login, or \
         (Keycloak) enable \"OAuth 2.0 Device Authorization Grant\" for the client",
    )?;
    let form = [("client_id", client_id), ("scope", SCOPE)];
    let (status, bytes) = post_form(client, endpoint, &form).await?;
    if !status.is_success() {
        let error = serde_json::from_slice::<TokenResponse>(&bytes)
            .map(|r| r.error())
            .unwrap_or_else(|_| format!("HTTP {status}"));
        return Err(format!("device login refused: {error}"));
    }
    let answer: DeviceResponse = serde_json::from_slice(&bytes)
        .map_err(|_| "identity provider: unexpected device login answer".to_owned())?;
    Ok(DeviceLogin {
        prompt: LoginPrompt {
            flow: LoginFlow::Device,
            url: answer.verification_uri,
            complete_url: answer.verification_uri_complete,
            user_code: Some(answer.user_code),
            expires_at: answer
                .expires_in
                .map(|secs| SystemTime::now() + Duration::from_secs(secs)),
        },
        device_code: Secret::new(answer.device_code),
        interval: Duration::from_secs(answer.interval.unwrap_or(5).max(1)),
    })
}

/// Poll until the user approved (or denied) the device login.
pub async fn poll_device(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    client_id: &str,
    login: &DeviceLogin,
) -> Result<TokenSet, String> {
    let mut interval = login.interval;
    loop {
        tokio::time::sleep(interval).await;
        let answer = token_request(
            client,
            &endpoints.token_endpoint,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", login.device_code.expose()),
                ("client_id", client_id),
            ],
        )
        .await?;
        match answer.error.as_deref() {
            None => return answer.tokens(SystemTime::now()),
            Some("authorization_pending") => {}
            Some("slow_down") => interval += Duration::from_secs(5),
            Some("expired_token") => return Err("the login code expired: press r to retry".into()),
            Some("access_denied") => return Err("the login was denied".into()),
            Some(_) => return Err(format!("login failed: {}", answer.error())),
        }
    }
}

/// A started browser login: open `prompt.url`, then [`finish_browser`].
#[derive(Debug)]
pub struct BrowserLogin {
    pub prompt: LoginPrompt,
    listener: tokio::net::TcpListener,
    redirect_uri: String,
    pkce: Pkce,
    state: String,
}

/// Listen on 127.0.0.1 (a free port) and build the authorization URL.
pub async fn start_browser(endpoints: &Endpoints, client_id: &str) -> Result<BrowserLogin, String> {
    use base64::Engine as _;
    let endpoint = endpoints
        .authorization_endpoint
        .as_deref()
        .ok_or("the identity provider doesn't offer browser login: use the device login")?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|err| format!("can't listen on 127.0.0.1 for the login: {err}"))?;
    let port = listener
        .local_addr()
        .map_err(|err| format!("can't listen on 127.0.0.1 for the login: {err}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let pkce = Pkce::from_random(random_bytes());
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes());
    let mut url = url::Url::parse(endpoint)
        .map_err(|_| "identity provider: invalid authorization endpoint".to_owned())?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("state", &state)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(BrowserLogin {
        prompt: LoginPrompt {
            flow: LoginFlow::Browser,
            url: url.into(),
            complete_url: None,
            user_code: None,
            expires_at: None,
        },
        listener,
        redirect_uri,
        pkce,
        state,
    })
}

/// Wait for the browser's redirect, check it, and exchange the code.
pub async fn finish_browser(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    client_id: &str,
    login: BrowserLogin,
) -> Result<TokenSet, String> {
    let code = loop {
        let (stream, _) = login
            .listener
            .accept()
            .await
            .map_err(|err| format!("login redirect: {err}"))?;
        // Browsers also ask for /favicon.ico and such: answer, keep waiting.
        if let Some(result) = serve_redirect(stream, &login.state).await {
            break result?;
        }
    };
    let answer = token_request(
        client,
        &endpoints.token_endpoint,
        &[
            ("grant_type", "authorization_code"),
            ("code", code.expose()),
            ("redirect_uri", &login.redirect_uri),
            ("client_id", client_id),
            ("code_verifier", login.pkce.verifier.expose()),
        ],
    )
    .await?;
    if answer.error.is_some() {
        return Err(format!("login failed: {}", answer.error()));
    }
    answer.tokens(SystemTime::now())
}

/// Read one request on the redirect listener and answer it. `None` when it
/// wasn't the redirect (e.g. `/favicon.ico`).
async fn serve_redirect(
    stream: tokio::net::TcpStream,
    state: &str,
) -> Option<Result<Secret, String>> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    let read =
        tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut request_line)).await;
    if !matches!(read, Ok(Ok(n)) if n > 0) {
        return None;
    }
    // "GET /callback?code=…&state=… HTTP/1.1"
    let target = request_line.split_whitespace().nth(1).unwrap_or("/");
    let url = url::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    let result = if url.path() != "/callback" {
        None
    } else {
        Some(code_from_redirect(&url, state))
    };
    let (status, page) = match &result {
        None => ("404 Not Found", "Not found."),
        Some(Ok(_)) => ("200 OK", "Logged in to Leeroy. You can close this tab."),
        Some(Err(_)) => (
            "400 Bad Request",
            "Leeroy's login failed: see Leeroy for why.",
        ),
    };
    let body = format!("<!doctype html><title>Leeroy</title><p>{page}</p>\n");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = reader.into_inner();
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
    result
}

/// The code from the redirect URL, after checking `state` (CSRF).
fn code_from_redirect(url: &url::Url, state: &str) -> Result<Secret, String> {
    let param = |name| jenkins::query_param(url, name);
    if let Some(error) = param("error") {
        let description = param("error_description").unwrap_or_default();
        return Err(format!("login failed: {error} {description}")
            .trim()
            .to_owned());
    }
    if param("state").as_deref() != Some(state) {
        return Err(
            "login failed: the answer doesn't belong to this login (state mismatch)".into(),
        );
    }
    param("code")
        .map(Secret::new)
        .ok_or_else(|| "login failed: no code in the redirect".into())
}

/// A new access token from a refresh token.
pub async fn refresh(
    client: &reqwest::Client,
    endpoints: &Endpoints,
    client_id: &str,
    refresh: &Secret,
) -> Result<TokenSet, String> {
    let answer = token_request(
        client,
        &endpoints.token_endpoint,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh.expose()),
            ("client_id", client_id),
        ],
    )
    .await?;
    if answer.error.is_some() {
        return Err(format!("the login expired: {}", answer.error()));
    }
    let mut tokens = answer.tokens(SystemTime::now())?;
    // Providers that don't rotate refresh tokens keep the old one valid.
    if tokens.refresh.is_none() {
        tokens.refresh = Some(refresh.clone());
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;

    async fn provider() -> (MockServer, Endpoints) {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(method("GET"))
            .and(path("/realms/acme/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "issuer": format!("{base}/realms/acme"),
                "token_endpoint": format!("{base}/token"),
                "authorization_endpoint": format!("{base}/auth"),
                "device_authorization_endpoint": format!("{base}/device"),
            })))
            .mount(&server)
            .await;
        let endpoints = discover(&reqwest::Client::new(), &format!("{base}/realms/acme/"))
            .await
            .unwrap();
        (server, endpoints)
    }

    fn tokens(access: &str, refresh: Option<&str>) -> ResponseTemplate {
        let mut body = serde_json::json!({ "access_token": access, "expires_in": 300 });
        if let Some(refresh) = refresh {
            body["refresh_token"] = refresh.into();
        }
        ResponseTemplate::new(200).set_body_json(body)
    }

    fn oauth_error(code: &str) -> ResponseTemplate {
        ResponseTemplate::new(400).set_body_json(serde_json::json!({ "error": code }))
    }

    #[tokio::test]
    async fn discovery_finds_endpoints_and_explains_a_wrong_issuer() {
        let (server, endpoints) = provider().await;
        assert_eq!(endpoints.token_endpoint, format!("{}/token", server.uri()));
        let err = discover(&reqwest::Client::new(), &format!("{}/nope", server.uri()))
            .await
            .unwrap_err();
        assert!(err.contains("realms/<realm>"), "{err}");
    }

    fn device_login(interval: u64) -> DeviceLogin {
        DeviceLogin {
            prompt: LoginPrompt {
                flow: LoginFlow::Device,
                url: "https://sso/device".into(),
                complete_url: None,
                user_code: Some("ABCD-EFGH".into()),
                expires_at: None,
            },
            device_code: Secret::new("dev-code"),
            interval: Duration::from_millis(interval),
        }
    }

    #[tokio::test]
    async fn device_login_shows_the_code() {
        let (server, endpoints) = provider().await;
        Mock::given(method("POST"))
            .and(path("/device"))
            .and(body_string_contains("client_id=leeroy"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "device_code": "dev-code",
                "user_code": "ABCD-EFGH",
                "verification_uri": "https://sso/device",
                "verification_uri_complete": "https://sso/device?user_code=ABCD-EFGH",
                "expires_in": 600,
                "interval": 5,
            })))
            .mount(&server)
            .await;
        let login = start_device(&reqwest::Client::new(), &endpoints, "leeroy")
            .await
            .unwrap();
        assert_eq!(login.prompt.user_code.as_deref(), Some("ABCD-EFGH"));
        assert_eq!(
            login.prompt.open_url(),
            "https://sso/device?user_code=ABCD-EFGH"
        );
        assert_eq!(login.interval, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn device_login_waits_while_pending() {
        let (server, endpoints) = provider().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(oauth_error("authorization_pending"))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("device_code=dev-code"))
            .respond_with(tokens("acc", Some("ref")))
            .mount(&server)
            .await;
        let tokens = poll_device(
            &reqwest::Client::new(),
            &endpoints,
            "leeroy",
            &device_login(1),
        )
        .await
        .unwrap();
        assert_eq!(tokens.access, Secret::new("acc"));
        assert_eq!(tokens.refresh, Some(Secret::new("ref")));
        assert!(tokens.expires_at.is_some());
    }

    #[tokio::test]
    async fn device_login_errors() {
        for (code, expected) in [
            ("expired_token", "expired"),
            ("access_denied", "denied"),
            ("invalid_client", "invalid_client"),
        ] {
            let (server, endpoints) = provider().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .respond_with(oauth_error(code))
                .mount(&server)
                .await;
            let err = poll_device(
                &reqwest::Client::new(),
                &endpoints,
                "leeroy",
                &device_login(1),
            )
            .await
            .unwrap_err();
            assert!(err.contains(expected), "{code}: {err}");
        }
    }

    #[tokio::test]
    async fn device_login_unavailable_says_how_to_enable_it() {
        let (_server, mut endpoints) = provider().await;
        endpoints.device_authorization_endpoint = None;
        let err = start_device(&reqwest::Client::new(), &endpoints, "leeroy")
            .await
            .unwrap_err();
        assert!(err.contains("Device Authorization Grant"), "{err}");
    }

    #[tokio::test]
    async fn browser_login_exchanges_the_code_with_the_verifier() {
        let (server, endpoints) = provider().await;
        let login = start_browser(&endpoints, "leeroy").await.unwrap();
        let auth = url::Url::parse(&login.prompt.url).unwrap();
        let param = |name: &str| {
            auth.query_pairs()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        assert_eq!(param("code_challenge_method"), "S256");
        let redirect = param("redirect_uri");
        assert!(redirect.starts_with("http://127.0.0.1:"), "{redirect}");
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=the-code"))
            .and(body_string_contains(format!(
                "code_verifier={}",
                login.pkce.verifier.expose()
            )))
            .respond_with(tokens("acc", Some("ref")))
            .mount(&server)
            .await;
        let state = param("state");
        let browser = tokio::spawn(async move {
            let http = reqwest::Client::new();
            // A stray request first: answered, the login keeps waiting.
            let favicon = http
                .get(redirect.replace("/callback", "/favicon.ico"))
                .send()
                .await
                .unwrap();
            assert_eq!(favicon.status(), 404);
            http.get(format!("{redirect}?code=the-code&state={state}"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        let tokens = finish_browser(&reqwest::Client::new(), &endpoints, "leeroy", login)
            .await
            .unwrap();
        assert_eq!(tokens.access, Secret::new("acc"));
        assert!(browser.await.unwrap().contains("You can close this tab"));
    }

    #[test]
    fn redirect_state_must_match() {
        let url = |q: &str| url::Url::parse(&format!("http://127.0.0.1/callback?{q}")).unwrap();
        assert_eq!(
            code_from_redirect(&url("code=c&state=s1"), "s1"),
            Ok(Secret::new("c"))
        );
        let err = code_from_redirect(&url("code=c&state=evil"), "s1").unwrap_err();
        assert!(err.contains("state mismatch"), "{err}");
        let err = code_from_redirect(&url("error=access_denied&state=s1"), "s1").unwrap_err();
        assert!(err.contains("access_denied"), "{err}");
    }

    #[tokio::test]
    async fn refresh_keeps_the_old_refresh_token_unless_rotated() {
        let (server, endpoints) = provider().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=old"))
            .respond_with(tokens("new-acc", None))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=rotating"))
            .respond_with(tokens("new-acc", Some("rotated")))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=stale"))
            .respond_with(oauth_error("invalid_grant"))
            .mount(&server)
            .await;
        let http = reqwest::Client::new();
        let kept = refresh(&http, &endpoints, "leeroy", &Secret::new("old"))
            .await
            .unwrap();
        assert_eq!(kept.refresh, Some(Secret::new("old")));
        let rotated = refresh(&http, &endpoints, "leeroy", &Secret::new("rotating"))
            .await
            .unwrap();
        assert_eq!(rotated.refresh, Some(Secret::new("rotated")));
        let err = refresh(&http, &endpoints, "leeroy", &Secret::new("stale"))
            .await
            .unwrap_err();
        assert!(err.contains("expired"), "{err}");
    }
}
