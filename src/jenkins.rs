//! Talking to a Jenkins instance.

use std::{fmt, time::Duration};

use serde::Deserialize;

use crate::config::redact_url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Everything needed to reach one Jenkins instance.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionConfig {
    pub url: String,
    /// Basic auth, used only when both username and token are set.
    pub credentials: Option<(String, String)>,
    /// Proxy URL to use, or `None` for a direct connection.
    pub proxy: Option<String>,
    /// Accept invalid TLS certificates. Insecure; off by default.
    pub skip_tls_verify: bool,
}

/// Hand-written so the token and proxy password never end up in logs.
impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("url", &self.url)
            .field(
                "credentials",
                &self
                    .credentials
                    .as_ref()
                    .map(|(user, _)| (user, "<redacted>")),
            )
            .field("proxy", &self.proxy.as_deref().map(redact_url))
            .field("skip_tls_verify", &self.skip_tls_verify)
            .finish()
    }
}

/// What we learned about the instance while connecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    /// From the `X-Jenkins` response header.
    pub version: Option<String>,
    /// The user Jenkins sees us as (`anonymous` without credentials).
    pub user: String,
}

#[derive(Deserialize)]
struct WhoAmI {
    name: String,
}

/// Build an HTTP client for `config`. Env proxy vars are never read here:
/// the proxy decision is made by the caller (see [`crate::proxy`]).
pub fn client(config: &ConnectionConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("leeroy/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .no_proxy()
        // Certs are checked against the OS trust store (rustls-platform-verifier),
        // so internal CAs installed system-wide work without this.
        .danger_accept_invalid_certs(config.skip_tls_verify);
    if let Some(proxy) = &config.proxy {
        let proxy = reqwest::Proxy::all(proxy)
            .map_err(|err| format!("invalid proxy {}: {}", redact_url(proxy), chain(&err)))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|err| format!("cannot create HTTP client: {}", chain(&err)))
}

/// Check that the instance is reachable and the credentials work.
///
/// Uses `whoAmI/api/json`, which also works for anonymous users, so a
/// successful result with user `anonymous` means "reachable, not logged in".
pub async fn check(config: &ConnectionConfig) -> Result<ServerInfo, String> {
    let client = client(config)?;
    let endpoint = api_url(&config.url, "whoAmI/api/json")?;
    let mut request = client.get(endpoint);
    if let Some((user, token)) = &config.credentials {
        request = request.basic_auth(user, Some(token));
    }

    let response = request.send().await.map_err(describe)?;
    let status = response.status();
    let version = response
        .headers()
        .get("X-Jenkins")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    match status.as_u16() {
        401 | 403 => return Err(format!("authentication failed (HTTP {})", status.as_u16())),
        _ if !status.is_success() => return Err(format!("unexpected response: HTTP {status}")),
        _ => {}
    }
    let who: WhoAmI = response
        .json()
        .await
        .map_err(|_| "unexpected response: not a Jenkins API (is the URL right?)".to_owned())?;
    Ok(ServerInfo {
        version,
        user: who.name,
    })
}

/// `base` joined with `path`, keeping any path prefix of `base`
/// (e.g. `https://host/jenkins`).
fn api_url(base: &str, path: &str) -> Result<url::Url, String> {
    let mut base = url::Url::parse(base).map_err(|err| format!("invalid Jenkins URL: {err}"))?;
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(path)
        .map_err(|err| format!("invalid Jenkins URL: {err}"))
}

/// A short, human-readable reason for a failed request.
fn describe(err: reqwest::Error) -> String {
    // The URL is already shown in the UI; drop it from the message.
    let err = err.without_url();
    if err.is_timeout() {
        "timed out".into()
    } else if err.is_connect() {
        let detail = chain(&err);
        match detail.split_once("invalid peer certificate: ") {
            Some((_, reason)) => format!(
                "untrusted TLS certificate ({reason}). Add its CA to the system trust \
                 store, or enable \"Skip TLS verify\" (insecure)"
            ),
            None => format!("cannot connect: {detail}"),
        }
    } else {
        chain(&err)
    }
}

/// An error and its sources, `outer: inner: innermost`, without repeats.
fn chain(err: &dyn std::error::Error) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(err) = source {
        let text = err.to_string();
        if !parts.iter().any(|p| p.contains(&text)) {
            parts.push(text);
        }
        source = err.source();
    }
    parts.join(": ")
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };

    use super::*;

    fn config(url: &str) -> ConnectionConfig {
        ConnectionConfig {
            url: url.into(),
            credentials: None,
            proxy: None,
            skip_tls_verify: false,
        }
    }

    fn whoami(name: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("X-Jenkins", "2.504.1")
            .set_body_json(serde_json::json!({ "name": name, "authenticated": true }))
    }

    #[test]
    fn api_url_keeps_path_prefix() {
        assert_eq!(
            api_url("https://host/jenkins", "whoAmI/api/json")
                .unwrap()
                .as_str(),
            "https://host/jenkins/whoAmI/api/json"
        );
        assert_eq!(
            api_url("https://host/", "whoAmI/api/json")
                .unwrap()
                .as_str(),
            "https://host/whoAmI/api/json"
        );
    }

    #[test]
    fn debug_redacts_secrets() {
        let config = ConnectionConfig {
            url: "https://ci".into(),
            credentials: Some(("me".into(), "s3cret".into())),
            proxy: Some("socks5h://u:hunter2@p:1080".into()),
            skip_tls_verify: false,
        };
        let debug = format!("{config:?}");
        assert!(
            !debug.contains("s3cret") && !debug.contains("hunter2"),
            "{debug}"
        );
    }

    #[tokio::test]
    async fn anonymous_check() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/whoAmI/api/json"))
            .respond_with(whoami("anonymous"))
            .mount(&server)
            .await;

        let info = check(&config(&server.uri())).await.unwrap();
        assert_eq!(
            info,
            ServerInfo {
                version: Some("2.504.1".into()),
                user: "anonymous".into()
            }
        );
    }

    #[tokio::test]
    async fn sends_basic_auth() {
        let server = MockServer::start().await;
        // base64("me:tok")
        Mock::given(path("/ci/whoAmI/api/json"))
            .and(header("authorization", "Basic bWU6dG9r"))
            .respond_with(whoami("me"))
            .mount(&server)
            .await;

        let mut config = config(&format!("{}/ci", server.uri()));
        config.credentials = Some(("me".into(), "tok".into()));
        assert_eq!(check(&config).await.unwrap().user, "me");
    }

    #[tokio::test]
    async fn auth_failure() {
        let server = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        assert_eq!(
            check(&config(&server.uri())).await.unwrap_err(),
            "authentication failed (HTTP 401)"
        );
    }

    #[tokio::test]
    async fn not_jenkins() {
        let server = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        let err = check(&config(&server.uri())).await.unwrap_err();
        assert!(err.contains("not a Jenkins API"), "{err}");
    }

    #[tokio::test]
    async fn connection_refused() {
        // Bind and drop to get a port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = check(&config(&format!("http://127.0.0.1:{port}")))
            .await
            .unwrap_err();
        assert!(err.starts_with("cannot connect"), "{err}");
    }

    /// The mock server acts as an HTTP proxy for a host that doesn't resolve:
    /// success proves the proxy is used and DNS happens on the proxy side.
    #[tokio::test]
    async fn goes_through_http_proxy() {
        let proxy = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .and(header("host", "jenkins.invalid"))
            .respond_with(whoami("anonymous"))
            .mount(&proxy)
            .await;

        let mut config = config("http://jenkins.invalid");
        config.proxy = Some(proxy.uri());
        assert_eq!(check(&config).await.unwrap().user, "anonymous");
    }
}
