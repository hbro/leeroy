//! How Leeroy authenticates: the auth modes, their settings, and the
//! credentials they produce. Pure: reading secrets is in [`crate::secrets`],
//! the OpenID Connect flows in [`crate::oidc`].
//!
//! - `headers` (the default): only the custom `[jenkins.headers]`, as before.
//! - `anonymous`: no credentials at all.
//! - `basic`: username + password or API token, the password from the OS
//!   keyring, a command (`pass show …`) or the config file.
//! - `oidc`: an access token from an OpenID Connect provider (Keycloak),
//!   sent as `Authorization: Bearer`, obtained with the device code or the
//!   browser (PKCE) flow and refreshed before it expires.

use std::{fmt, time::SystemTime};

/// A password or token. `Debug` never shows it (actions and effects are
/// logged); read it with [`Secret::expose`].
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(••••)")
    }
}

/// `auth.mode` values, in the order Enter cycles through them (the first is
/// the default: the key absent).
pub const MODE_VALUES: [&str; 4] = ["headers", "anonymous", "basic", "oidc"];
/// `auth.password_store` values.
pub const PASSWORD_STORE_VALUES: [&str; 3] = ["keyring", "command", "file"];
/// `auth.login_flow` values.
pub const LOGIN_FLOW_VALUES: [&str; 2] = ["device", "browser"];
/// `auth.token_store` values.
pub const TOKEN_STORE_VALUES: [&str; 3] = ["keyring", "file", "memory"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// Only the custom headers (what Leeroy did before auth modes).
    Headers,
    Anonymous,
    Basic,
    Oidc,
}

impl AuthMode {
    /// From a setting value; unset or unknown = [`AuthMode::Headers`].
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("anonymous") => AuthMode::Anonymous,
            Some("basic") => AuthMode::Basic,
            Some("oidc") => AuthMode::Oidc,
            _ => AuthMode::Headers,
        }
    }

    /// Shown in the settings view and the instance overlay.
    pub fn label(self) -> &'static str {
        match self {
            AuthMode::Headers => "headers only",
            AuthMode::Anonymous => "anonymous",
            AuthMode::Basic => "username + password",
            AuthMode::Oidc => "Keycloak / OpenID Connect",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStore {
    Keyring,
    Command,
    File,
}

impl PasswordStore {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("command") => PasswordStore::Command,
            Some("file") => PasswordStore::File,
            _ => PasswordStore::Keyring,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginFlow {
    /// Show a code to enter at a URL, in any browser (works over SSH).
    Device,
    /// Open the browser and receive the redirect on 127.0.0.1 (PKCE).
    Browser,
}

impl LoginFlow {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("browser") => LoginFlow::Browser,
            _ => LoginFlow::Device,
        }
    }

    pub fn other(self) -> Self {
        match self {
            LoginFlow::Device => LoginFlow::Browser,
            LoginFlow::Browser => LoginFlow::Device,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStore {
    Keyring,
    File,
    /// Kept only while Leeroy runs: log in at every start.
    Memory,
}

impl TokenStore {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("file") => TokenStore::File,
            Some("memory") => TokenStore::Memory,
            _ => TokenStore::Keyring,
        }
    }

    /// Where the login is written (`None`: memory, nowhere).
    pub fn secret_store(self) -> Option<SecretStore> {
        match self {
            TokenStore::Keyring => Some(SecretStore::Keyring),
            TokenStore::File => Some(SecretStore::TokensFile),
            TokenStore::Memory => None,
        }
    }
}

/// Basic auth settings (the password itself is resolved separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicConfig {
    /// `None`: not set yet (connecting fails with a hint).
    pub username: Option<String>,
    pub store: PasswordStore,
    pub command: Option<String>,
}

/// What basic auth still needs before it can connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    Username,
    PasswordCommand,
    Password,
}

impl Missing {
    pub fn message(self) -> &'static str {
        match self {
            Missing::Username => "basic auth: set the Username in the settings (0)",
            Missing::PasswordCommand => "basic auth: set the Password command in the settings (0)",
            Missing::Password => "basic auth: set the Password in the settings (0)",
        }
    }
}

impl BasicConfig {
    /// What's missing, `password_known`: a password is given without its
    /// store (env var, config file, typed). The keyring may hold one: that
    /// only shows when asking it.
    pub fn missing(&self, password_known: bool) -> Option<Missing> {
        if self.username.is_none() {
            Some(Missing::Username)
        } else if password_known {
            None
        } else {
            match self.store {
                PasswordStore::Command if self.command.is_none() => Some(Missing::PasswordCommand),
                PasswordStore::File => Some(Missing::Password),
                _ => None,
            }
        }
    }
}

/// OpenID Connect settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcConfig {
    /// Issuer URL, e.g. `https://sso.example.com/realms/acme` for Keycloak.
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    pub flow: LoginFlow,
    pub store: TokenStore,
}

impl OidcConfig {
    /// Issuer and client id, or why they can't be used yet.
    pub fn issuer_and_client(&self) -> Result<(&str, &str), String> {
        match (self.issuer.as_deref(), self.client_id.as_deref()) {
            (Some(issuer), Some(client)) => Ok((issuer, client)),
            _ => Err("set the Issuer URL and Client ID in the settings (0)".into()),
        }
    }

    /// The keyring account / tokens file key of this login.
    pub fn account(&self) -> Result<String, String> {
        let (issuer, client) = self.issuer_and_client()?;
        Ok(token_account(issuer, client))
    }
}

/// The auth part of the settings: part of what identifies a connection, so
/// changing it reconnects. Holds no secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthConfig {
    Headers,
    Anonymous,
    Basic(BasicConfig),
    Oidc(OidcConfig),
}

impl AuthConfig {
    pub fn mode(&self) -> AuthMode {
        match self {
            AuthConfig::Headers => AuthMode::Headers,
            AuthConfig::Anonymous => AuthMode::Anonymous,
            AuthConfig::Basic(_) => AuthMode::Basic,
            AuthConfig::Oidc(_) => AuthMode::Oidc,
        }
    }

    /// Whether credentials must be resolved (password, token) before
    /// connecting.
    pub fn needs_credentials(&self) -> bool {
        matches!(self, AuthConfig::Basic(_) | AuthConfig::Oidc(_))
    }
}

/// What Leeroy sends to authenticate, resolved at connect time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Credentials {
    /// Nothing besides the custom headers.
    #[default]
    None,
    Basic {
        username: String,
        password: Secret,
    },
    Bearer(Secret),
}

impl Credentials {
    /// The `Authorization` header value, if any.
    pub fn authorization(&self) -> Option<Secret> {
        use base64::Engine as _;
        match self {
            Credentials::None => None,
            Credentials::Basic { username, password } => {
                let pair = format!("{username}:{}", password.expose());
                Some(Secret::new(format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(pair)
                )))
            }
            Credentials::Bearer(token) => Some(Secret::new(format!("Bearer {}", token.expose()))),
        }
    }
}

/// Tokens from the OpenID Connect provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSet {
    pub access: Secret,
    /// To get a new access token without logging in again.
    pub refresh: Option<Secret>,
    /// When the access token expires (`None`: not said).
    pub expires_at: Option<SystemTime>,
    /// When to get a new one (`None`: never ahead, only after a failure).
    pub refresh_at: Option<SystemTime>,
}

/// Refresh this long before the access token expires (or halfway through
/// its lifetime, if that's later: short-lived tokens).
pub const REFRESH_AHEAD: std::time::Duration = std::time::Duration::from_secs(30);

impl TokenSet {
    /// Tokens issued at `now`, the access token valid for `lifetime`.
    pub fn issued(
        access: Secret,
        refresh: Option<Secret>,
        now: SystemTime,
        lifetime: Option<std::time::Duration>,
    ) -> Self {
        Self {
            access,
            refresh,
            expires_at: lifetime.map(|l| now + l),
            refresh_at: lifetime.map(|l| now + l.saturating_sub(REFRESH_AHEAD).max(l / 2)),
        }
    }

    /// Whether the access token should be refreshed at `now`.
    pub fn refresh_due(&self, now: SystemTime) -> bool {
        self.refresh_at.is_some_and(|at| now >= at)
    }
}

/// Where a secret is written ([`crate::app::Effect::StoreSecret`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStore {
    Keyring,
    /// The tokens file in Leeroy's state directory.
    TokensFile,
}

/// What resolving credentials before connecting came up with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Basic auth: ready to connect.
    Credentials(Credentials),
    /// OpenID Connect: a usable (possibly just refreshed) access token.
    Tokens(TokenSet),
    /// OpenID Connect: no usable login; the user has to log in.
    LoginNeeded,
}

/// Keyring account (and tokens file key) for a basic-auth password.
pub fn password_account(jenkins_url: &str, username: &str) -> String {
    format!("basic:{}:{username}", jenkins_url.trim_end_matches('/'))
}

/// Keyring account (and tokens file key) for OpenID Connect tokens.
pub fn token_account(issuer: &str, client_id: &str) -> String {
    format!("oidc:{}:{client_id}", issuer.trim_end_matches('/'))
}

/// Shown on the login overlay while logging in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginPrompt {
    pub flow: LoginFlow,
    /// Where to log in: the verification URL (device) or the authorization
    /// URL (browser).
    pub url: String,
    /// The URL with the code filled in, if the provider sends one.
    pub complete_url: Option<String>,
    /// The code to enter (device flow).
    pub user_code: Option<String>,
    pub expires_at: Option<SystemTime>,
}

impl LoginPrompt {
    /// What `o` opens: the URL with the code when there is one.
    pub fn open_url(&self) -> &str {
        self.complete_url.as_deref().unwrap_or(&self.url)
    }
}

/// PKCE (RFC 7636): a random verifier and its S256 challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pkce {
    pub verifier: Secret,
    pub challenge: String,
}

impl Pkce {
    pub fn from_random(bytes: [u8; 32]) -> Self {
        use base64::Engine as _;
        use sha2::Digest as _;
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let verifier = engine.encode(bytes);
        let challenge = engine.encode(sha2::Sha256::digest(verifier.as_bytes()));
        Self {
            verifier: Secret::new(verifier),
            challenge,
        }
    }
}

/// Random URL-safe text (for PKCE verifiers and `state`).
pub fn random_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("no OS random source");
    bytes
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn secrets_never_show_in_debug() {
        let creds = Credentials::Basic {
            username: "me".into(),
            password: Secret::new("s3cret"),
        };
        let shown = format!("{creds:?} {:?}", creds.authorization());
        assert!(!shown.contains("s3cret"), "{shown}");
        assert!(shown.contains("me"), "{shown}");
        let tokens = TokenSet {
            access: Secret::new("tok-a1"),
            refresh: Some(Secret::new("tok-r2")),
            expires_at: None,
            refresh_at: None,
        };
        let shown = format!("{tokens:?}");
        assert!(!shown.contains("tok-"), "{shown}");
    }

    #[test]
    fn authorization_headers() {
        let basic = Credentials::Basic {
            username: "me".into(),
            password: Secret::new("tok"),
        };
        assert_eq!(basic.authorization().unwrap().expose(), "Basic bWU6dG9r");
        let bearer = Credentials::Bearer(Secret::new("abc"));
        assert_eq!(bearer.authorization().unwrap().expose(), "Bearer abc");
        assert_eq!(Credentials::None.authorization(), None);
    }

    #[test]
    fn modes_parse_with_headers_as_default() {
        for (value, mode) in MODE_VALUES.iter().zip([
            AuthMode::Headers,
            AuthMode::Anonymous,
            AuthMode::Basic,
            AuthMode::Oidc,
        ]) {
            assert_eq!(AuthMode::parse(Some(value)), mode);
        }
        assert_eq!(AuthMode::parse(None), AuthMode::Headers);
        assert_eq!(PasswordStore::parse(None), PasswordStore::Keyring);
        assert_eq!(LoginFlow::parse(None), LoginFlow::Device);
        assert_eq!(TokenStore::parse(None), TokenStore::Keyring);
    }

    #[test]
    fn refresh_is_due_shortly_before_expiry() {
        let issued = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let at = |secs| issued + Duration::from_secs(secs);
        let tokens = |lifetime: Option<u64>| {
            TokenSet::issued(
                Secret::new("a"),
                None,
                issued,
                lifetime.map(Duration::from_secs),
            )
        };
        // Keycloak's default 5 minutes: 30 s before.
        assert!(!tokens(Some(300)).refresh_due(at(269)));
        assert!(tokens(Some(300)).refresh_due(at(270)));
        // Short-lived: halfway, never in a tight loop.
        assert!(!tokens(Some(20)).refresh_due(at(9)));
        assert!(tokens(Some(20)).refresh_due(at(10)));
        assert!(!tokens(None).refresh_due(at(100_000)));
    }

    #[test]
    fn pkce_matches_rfc_7636_example() {
        // RFC 7636 appendix B.
        let bytes = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let pkce = Pkce::from_random(bytes);
        assert_eq!(
            pkce.verifier.expose(),
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        );
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn accounts_ignore_trailing_slashes() {
        assert_eq!(
            password_account("https://ci.example.com/", "me"),
            "basic:https://ci.example.com:me"
        );
        assert_eq!(
            token_account("https://sso.example.com/realms/acme/", "leeroy"),
            "oidc:https://sso.example.com/realms/acme:leeroy"
        );
    }
}
