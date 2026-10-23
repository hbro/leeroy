//! Login detection: how does this Jenkins URL want to be logged in to?
//! Requests are made without credentials and redirects are followed by hand,
//! so every hop can be looked at: a Jenkins that answers anonymously, one that
//! asks for basic auth or has its own SSO plugin, or a single sign-on proxy
//! in front of it that sends browsers to Keycloak. `classify` is pure; the
//! rest is IO for the executor.

use crate::{
    auth::MODE_VALUES,
    config::{SettingKey, redact_url},
    jenkins::{self, ConnectionConfig},
    oidc,
};

/// At most this many redirects per probe.
const MAX_HOPS: usize = 10;

/// One answer on the way (a redirect, or the final one).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hop {
    url: url::Url,
    status: u16,
    location: Option<String>,
    www_authenticate: Option<String>,
    /// `X-Jenkins`: the answer came from Jenkins itself.
    jenkins: Option<String>,
}

/// What detection found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detection {
    /// Jenkins lets anyone read: no login needed.
    Anonymous { version: Option<String> },
    /// Jenkins itself wants a login; `sso`: through its OpenID Connect or
    /// Keycloak plugin (then the API takes an API token, not the SSO login).
    JenkinsLogin {
        version: Option<String>,
        sso: Option<Sso>,
    },
    /// HTTP basic auth, asked for by Jenkins or a server in front of it.
    Basic {
        realm: Option<String>,
        jenkins: bool,
    },
    /// A single sign-on proxy in front of Jenkins (`proxy`: which one, if
    /// recognised) that sends browsers to `sso`.
    SsoProxy { sso: Sso, proxy: Option<String> },
    /// Nothing recognisable.
    Unknown { reason: String },
}

/// The identity provider a login is sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sso {
    /// Where the browser login happens (scheme + host).
    pub login_at: String,
    /// The OpenID Connect issuer (for Keycloak: the realm URL), if known.
    pub issuer: Option<String>,
    /// The client the redirect named (that of Jenkins or the proxy).
    pub client_id: Option<String>,
    /// Whether Leeroy could use that client for a device login.
    pub device: DeviceCheck,
}

impl Sso {
    /// Keycloak (its login path names the realm, so the issuer is known).
    pub fn keycloak(&self) -> bool {
        self.issuer.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceCheck {
    NotChecked,
    /// The provider started a device login for the client: usable as is.
    Works,
    /// It refused (the provider's reason): the client is confidential or
    /// has device login off.
    Refused(String),
    /// The provider has no device login at all.
    Unavailable,
}

impl Detection {
    /// The settings this suggests (applied to the draft, to review and save).
    pub fn suggested_settings(&self) -> Vec<(SettingKey, Option<String>)> {
        let mode = |m: &str| {
            let value = (m != MODE_VALUES[0]).then(|| m.to_owned());
            (SettingKey::AuthMode, value)
        };
        match self {
            Detection::Anonymous { .. } => vec![mode("anonymous")],
            Detection::JenkinsLogin { .. } | Detection::Basic { .. } => vec![mode("basic")],
            Detection::SsoProxy { sso, .. } => {
                let mut settings = vec![mode("oidc")];
                if let Some(issuer) = &sso.issuer {
                    settings.push((SettingKey::AuthIssuer, Some(issuer.clone())));
                }
                if let Some(client) = &sso.client_id {
                    settings.push((SettingKey::AuthClientId, Some(client.clone())));
                }
                if sso.device == DeviceCheck::Works {
                    settings.push((SettingKey::AuthLoginFlow, None));
                }
                settings
            }
            Detection::Unknown { .. } => Vec::new(),
        }
    }

    /// Where to create a Jenkins API token, when that's the way to log in
    /// (credentials in `jenkins_url` left out).
    pub fn token_page(&self, jenkins_url: &str) -> Option<String> {
        if !matches!(self, Detection::JenkinsLogin { .. }) {
            return None;
        }
        let mut url = jenkins::api_url(jenkins_url, "me/security/").ok()?;
        url.set_username("").ok()?;
        url.set_password(None).ok()?;
        Some(url.into())
    }
}

/// Request `url` without credentials, following redirects by hand.
async fn follow(client: &reqwest::Client, url: url::Url) -> Result<Vec<Hop>, String> {
    let mut hops = Vec::new();
    let mut next = url;
    for _ in 0..MAX_HOPS {
        let response = client
            .get(next.clone())
            .send()
            .await
            .map_err(jenkins::describe)?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let hop = Hop {
            url: next.clone(),
            status: response.status().as_u16(),
            location: header("Location"),
            www_authenticate: header("WWW-Authenticate"),
            jenkins: header("X-Jenkins"),
        };
        let redirect = (300..400).contains(&hop.status);
        let target = hop.location.as_deref().and_then(|l| next.join(l).ok());
        hops.push(hop);
        match target {
            // The identity provider's login page itself isn't needed.
            Some(target) if redirect && sso_from_url(&target).is_none() => next = target,
            _ => return Ok(hops),
        }
    }
    Ok(hops)
}

/// The identity provider a URL points at, if it's an OpenID Connect
/// authorization request (`client_id` + `response_type`, or Keycloak's path).
fn sso_from_url(url: &url::Url) -> Option<Sso> {
    let param = |name| jenkins::query_param(url, name);
    let path = url.path();
    let keycloak_auth = path.find("/protocol/openid-connect/auth");
    let oidc_request = param("client_id").is_some() && param("response_type").is_some();
    if keycloak_auth.is_none() && !oidc_request {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    let issuer = keycloak_auth
        .filter(|_| path.contains("/realms/"))
        .map(|at| format!("{origin}{}", &path[..at]));
    Some(Sso {
        login_at: origin,
        issuer,
        client_id: param("client_id"),
        device: DeviceCheck::NotChecked,
    })
}

/// The identity provider the hops redirect to, if any.
fn sso_redirect_in(hops: &[Hop]) -> Option<Sso> {
    hops.iter()
        .filter_map(|h| h.location.as_deref().and_then(|l| h.url.join(l).ok()))
        .find_map(|url| sso_from_url(&url))
}

/// A recognised SSO proxy, from the paths it redirected through.
fn proxy_of(hops: &[Hop]) -> Option<String> {
    hops.iter().find_map(|hop| {
        let path = hop.url.path();
        if path.starts_with("/oauth2/") {
            Some("oauth2-proxy".to_owned())
        } else if path.starts_with("/oauth/authorize") || path.starts_with("/oauth/callback") {
            Some("Keycloak Gatekeeper / Louketo".to_owned())
        } else {
            None
        }
    })
}

/// What the hops of `whoAmI/api/json` (always open to anonymous users on
/// Jenkins) say. `None`: Jenkins answered; whether it needs a login is
/// probed next.
fn classify(hops: &[Hop]) -> Option<Detection> {
    let last = hops.last()?;
    if let Some(sso) = sso_redirect_in(hops) {
        // Jenkins never redirects whoAmI: someone in front of it does.
        return Some(Detection::SsoProxy {
            sso,
            proxy: proxy_of(hops),
        });
    }
    if last.status == 401 {
        let challenge = last.www_authenticate.clone().unwrap_or_default();
        if challenge.to_ascii_lowercase().starts_with("basic") {
            let realm = challenge
                .split("realm=")
                .nth(1)
                .map(|r| r.trim_matches('"').to_owned());
            return Some(Detection::Basic {
                realm,
                jenkins: jenkins::answered_by_jenkins(
                    last.jenkins.is_some(),
                    last.www_authenticate.as_deref(),
                ),
            });
        }
        return Some(Detection::Unknown {
            reason: format!(
                "HTTP 401{}",
                if challenge.is_empty() {
                    String::new()
                } else {
                    format!(", asks for: {challenge}")
                }
            ),
        });
    }
    if (200..300).contains(&last.status) && last.jenkins.is_some() {
        return None;
    }
    Some(Detection::Unknown {
        reason: match (last.status, &last.jenkins) {
            (200..300, None) => format!(
                "{} answered, but it isn't Jenkins (is the URL right?)",
                redact_url(last.url.as_str())
            ),
            (status, _) => format!("HTTP {status} from {}", redact_url(last.url.as_str())),
        },
    })
}

/// Detect how `config.url` wants to be logged in to (credentials and
/// headers of `config` aren't used).
pub async fn detect(config: &ConnectionConfig) -> Result<Detection, String> {
    let client = jenkins::client_with(config, reqwest::redirect::Policy::none())?;
    let join = |path| jenkins::api_url(&config.url, path);
    let hops = follow(&client, join("whoAmI/api/json")?).await?;
    let version = hops.last().and_then(|h| h.jenkins.clone());
    let mut detection = match classify(&hops) {
        Some(detection) => detection,
        None => {
            // Jenkins answers: can anonymous users read?
            let read = follow(&client, join("api/json?tree=mode")?).await?;
            if read.last().is_some_and(|h| (200..300).contains(&h.status)) {
                Detection::Anonymous { version }
            } else {
                // Its SSO plugins (OpenID Connect, Keycloak) start here.
                let login = follow(&client, join("securityRealm/commenceLogin?from=%2F")?).await?;
                Detection::JenkinsLogin {
                    version,
                    sso: sso_redirect_in(&login),
                }
            }
        }
    };
    if let Detection::SsoProxy { sso, .. } = &mut detection {
        sso.device = check_device(config, sso).await;
    }
    Ok(detection)
}

/// Whether the provider would start a device login for the client found.
/// (Starting one is harmless: unused, it just expires.)
async fn check_device(config: &ConnectionConfig, sso: &Sso) -> DeviceCheck {
    let (Some(issuer), Some(client_id)) = (&sso.issuer, &sso.client_id) else {
        return DeviceCheck::NotChecked;
    };
    let Ok(client) = jenkins::client(config) else {
        return DeviceCheck::NotChecked;
    };
    let endpoints = match oidc::discover(&client, issuer).await {
        Ok(endpoints) => endpoints,
        Err(err) => return DeviceCheck::Refused(err),
    };
    if endpoints.device_authorization_endpoint.is_none() {
        return DeviceCheck::Unavailable;
    }
    match oidc::start_device(&client, &endpoints, client_id).await {
        Ok(_) => DeviceCheck::Works,
        Err(err) => DeviceCheck::Refused(err),
    }
}

/// The message for the admin when the client found can't be used.
pub fn admin_request(sso: &Sso, proxy: Option<&str>) -> Vec<String> {
    let issuer = sso.issuer.as_deref().unwrap_or("the realm");
    let accept = match proxy {
        Some("oauth2-proxy") => format!(
            "oauth2-proxy accepting its tokens: --skip-jwt-bearer-tokens \
             --extra-jwt-issuers={issuer}=leeroy."
        ),
        Some(proxy) => format!("{proxy} accepting its tokens as bearer tokens."),
        None => "The proxy in front of Jenkins accepting its tokens as bearer tokens.".into(),
    };
    vec![
        format!(
            "1. A public client for Leeroy, e.g. \"leeroy\", in {issuer}: client \
             authentication off, \"OAuth 2.0 Device Authorization Grant\" on."
        ),
        format!("2. {accept}"),
    ]
}

#[cfg(test)]
mod tests {
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    use super::*;
    use crate::auth;

    fn hop(url: &str, status: u16, location: Option<&str>, jenkins: bool) -> Hop {
        Hop {
            url: url::Url::parse(url).unwrap(),
            status,
            location: location.map(str::to_owned),
            www_authenticate: None,
            jenkins: jenkins.then(|| "2.504.1".to_owned()),
        }
    }

    #[test]
    fn keycloak_issuer_and_client_from_the_login_redirect() {
        let url = url::Url::parse(
            "https://sso.example.com/realms/acme/protocol/openid-connect/auth\
             ?client_id=ci-proxy&response_type=code&redirect_uri=https%3A%2F%2Fci%2Foauth2%2Fcallback",
        )
        .unwrap();
        let sso = sso_from_url(&url).unwrap();
        assert_eq!(
            sso.issuer.as_deref(),
            Some("https://sso.example.com/realms/acme")
        );
        assert_eq!(sso.client_id.as_deref(), Some("ci-proxy"));
        assert!(sso.keycloak());
        // Keycloak before 17: /auth/realms/…
        let old = url::Url::parse(
            "https://sso.example.com:8443/auth/realms/acme/protocol/openid-connect/auth?client_id=x",
        )
        .unwrap();
        assert_eq!(
            sso_from_url(&old).unwrap().issuer.as_deref(),
            Some("https://sso.example.com:8443/auth/realms/acme")
        );
        assert_eq!(
            sso_from_url(&url::Url::parse("https://ci/login").unwrap()),
            None
        );
    }

    #[test]
    fn oauth2_proxy_in_front() {
        let hops = [
            hop(
                "https://ci/whoAmI/api/json",
                302,
                Some("/oauth2/start?rd=%2FwhoAmI"),
                false,
            ),
            hop(
                "https://ci/oauth2/start?rd=%2FwhoAmI",
                302,
                Some(
                    "https://sso/realms/acme/protocol/openid-connect/auth?client_id=ci&response_type=code",
                ),
                false,
            ),
        ];
        assert!(matches!(
            classify(&hops),
            Some(Detection::SsoProxy { sso, proxy: Some(p) })
                if p == "oauth2-proxy" && sso.client_id.as_deref() == Some("ci")
        ));
    }

    #[test]
    fn basic_challenge_and_jenkins_answers() {
        let mut challenge = hop("https://ci/whoAmI/api/json", 401, None, false);
        challenge.www_authenticate = Some("Basic realm=\"corp\"".into());
        assert_eq!(
            classify(&[challenge]),
            Some(Detection::Basic {
                realm: Some("corp".into()),
                jenkins: false
            })
        );
        assert_eq!(
            classify(&[hop("https://ci/whoAmI/api/json", 200, None, true)]),
            None
        );
        assert!(matches!(
            classify(&[hop("https://ci/whoAmI/api/json", 200, None, false)]),
            Some(Detection::Unknown { reason }) if reason.contains("isn't Jenkins")
        ));
    }

    #[test]
    fn token_page_leaves_url_credentials_out() {
        let found = Detection::JenkinsLogin {
            version: None,
            sso: None,
        };
        assert_eq!(
            found.token_page("https://me:s3cret@ci.example.com/jenkins"),
            Some("https://ci.example.com/jenkins/me/security/".into())
        );
        assert_eq!(
            Detection::Anonymous { version: None }.token_page("https://ci"),
            None
        );
    }

    #[test]
    fn suggestions() {
        let sso = Sso {
            login_at: "https://sso".into(),
            issuer: Some("https://sso/realms/acme".into()),
            client_id: Some("ci".into()),
            device: DeviceCheck::Refused("unauthorized_client".into()),
        };
        assert_eq!(
            Detection::SsoProxy { sso, proxy: None }.suggested_settings(),
            vec![
                (SettingKey::AuthMode, Some("oidc".into())),
                (
                    SettingKey::AuthIssuer,
                    Some("https://sso/realms/acme".into())
                ),
                (SettingKey::AuthClientId, Some("ci".into())),
            ]
        );
        assert_eq!(
            Detection::JenkinsLogin {
                version: None,
                sso: None
            }
            .suggested_settings(),
            vec![(SettingKey::AuthMode, Some("basic".into()))]
        );
    }

    async fn jenkins_mock(read: u16) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(ResponseTemplate::new(200).insert_header("X-Jenkins", "2.504.1"))
            .mount(&server)
            .await;
        Mock::given(path("/api/json"))
            .respond_with(ResponseTemplate::new(read).insert_header("X-Jenkins", "2.504.1"))
            .mount(&server)
            .await;
        server
    }

    fn config(url: &str) -> ConnectionConfig {
        ConnectionConfig {
            url: url.into(),
            headers: Vec::new(),
            proxy: None,
            skip_tls_verify: false,
            auth: auth::AuthConfig::Headers,
            credentials: auth::Credentials::None,
        }
    }

    #[tokio::test]
    async fn anonymous_read() {
        let server = jenkins_mock(200).await;
        assert_eq!(
            detect(&config(&server.uri())).await,
            Ok(Detection::Anonymous {
                version: Some("2.504.1".into())
            })
        );
    }

    #[tokio::test]
    async fn jenkins_with_its_own_sso_plugin() {
        let server = jenkins_mock(403).await;
        Mock::given(path("/securityRealm/commenceLogin"))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "Location",
                "https://sso.example.com/realms/acme/protocol/openid-connect/auth?client_id=jenkins&response_type=code",
            ))
            .mount(&server)
            .await;
        assert!(matches!(
            detect(&config(&server.uri())).await,
            Ok(Detection::JenkinsLogin { sso: Some(sso), .. }) if sso.client_id.as_deref() == Some("jenkins")
        ));
    }

    #[tokio::test]
    async fn sso_proxy_whose_client_refuses_device_login() {
        let server = MockServer::start().await;
        let base = server.uri();
        Mock::given(path("/whoAmI/api/json"))
            .respond_with(
                ResponseTemplate::new(302).insert_header("Location", "/oauth2/start?rd=%2F"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/oauth2/start"))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "Location",
                format!("{base}/realms/acme/protocol/openid-connect/auth?client_id=ci-proxy&response_type=code"),
            ))
            .mount(&server)
            .await;
        Mock::given(path("/realms/acme/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "token_endpoint": format!("{base}/token"),
                "device_authorization_endpoint": format!("{base}/device"),
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/device"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": "unauthorized_client",
                "error_description": "Invalid client or Invalid client credentials",
            })))
            .mount(&server)
            .await;
        let detection = detect(&config(&base)).await.unwrap();
        let Detection::SsoProxy { sso, proxy } = detection else {
            panic!("{detection:?}");
        };
        assert_eq!(proxy.as_deref(), Some("oauth2-proxy"));
        assert_eq!(sso.issuer, Some(format!("{base}/realms/acme")));
        assert!(
            matches!(sso.device, DeviceCheck::Refused(why) if why.contains("unauthorized_client"))
        );
    }
}
