//! Which proxy applies to requests to Jenkins.
//!
//! Leeroy's own `proxy.url` setting (or `$LEEROY_PROXY_URL`) always wins and
//! ignores `NO_PROXY`. Without it, the conventional env vars are used, like
//! curl does: `https_proxy`/`http_proxy` by target scheme, then `all_proxy`
//! (lowercase checked before uppercase), unless `no_proxy` matches the host.

use std::ffi::OsString;

/// Snapshot of the conventional proxy env vars, taken at startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyEnv {
    vars: Vec<(&'static str, String)>,
}

const VARS: &[&str] = &[
    "https_proxy",
    "HTTPS_PROXY",
    "http_proxy",
    "HTTP_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "no_proxy",
    "NO_PROXY",
];

/// The system proxy decision for one target URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemProxy {
    /// Use `url`, taken from env var `var`.
    Proxy { var: &'static str, url: String },
    /// A proxy is configured, but `var` (no_proxy) excludes the target host.
    Bypassed { var: &'static str },
    /// No proxy env vars apply.
    None,
}

impl ProxyEnv {
    /// Read the proxy env vars. Empty or non-UTF-8 values count as unset.
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Self {
        let vars = VARS
            .iter()
            .filter_map(|&name| {
                let value = env(name)?.into_string().ok()?;
                (!value.trim().is_empty()).then(|| (name, value.trim().to_owned()))
            })
            .collect();
        Self { vars }
    }

    fn first(&self, names: &[&'static str]) -> Option<(&'static str, &str)> {
        names.iter().find_map(|&name| {
            self.vars
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (name, v.as_str()))
        })
    }

    /// The system proxy for requests to `target` (a URL). Without a target,
    /// an https URL with an unknown host is assumed.
    pub fn for_target(&self, target: Option<&str>) -> SystemProxy {
        let parsed = target.and_then(|t| url::Url::parse(t).ok());
        let scheme = parsed.as_ref().map_or("https", |u| u.scheme());
        let host = parsed.as_ref().and_then(|u| u.host_str());

        let by_scheme: &[&'static str] = if scheme == "http" {
            &["http_proxy", "HTTP_PROXY", "all_proxy", "ALL_PROXY"]
        } else {
            &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"]
        };
        let Some((var, url)) = self.first(by_scheme) else {
            return SystemProxy::None;
        };
        if let (Some(host), Some((no_var, no_proxy))) =
            (host, self.first(&["no_proxy", "NO_PROXY"]))
            && no_proxy_matches(no_proxy, host)
        {
            return SystemProxy::Bypassed { var: no_var };
        }
        SystemProxy::Proxy {
            var,
            url: url.to_owned(),
        }
    }
}

/// curl-style `NO_PROXY` matching: comma-separated host names or domain
/// suffixes (a leading `.` is optional), or `*` for everything. Ports and
/// CIDR ranges are not supported.
fn no_proxy_matches(no_proxy: &str, host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    no_proxy
        .split(',')
        .map(|entry| entry.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry == "*" || host == entry || host.ends_with(&format!(".{entry}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_env(vars: &[(&str, &str)]) -> ProxyEnv {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        ProxyEnv::from_env(move |name| {
            vars.iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| OsString::from(v))
        })
    }

    fn proxy(var: &'static str, url: &str) -> SystemProxy {
        SystemProxy::Proxy {
            var,
            url: url.into(),
        }
    }

    #[test]
    fn nothing_set() {
        assert_eq!(
            proxy_env(&[]).for_target(Some("https://ci")),
            SystemProxy::None
        );
    }

    #[test]
    fn picks_var_by_scheme_then_all_proxy() {
        let env = proxy_env(&[
            ("HTTPS_PROXY", "http://secure:3128"),
            ("http_proxy", "http://plain:3128"),
            ("ALL_PROXY", "socks5h://all:1080"),
        ]);
        assert_eq!(
            env.for_target(Some("https://ci.example.com")),
            proxy("HTTPS_PROXY", "http://secure:3128")
        );
        assert_eq!(
            env.for_target(Some("http://ci.example.com")),
            proxy("http_proxy", "http://plain:3128")
        );

        let only_all = proxy_env(&[("ALL_PROXY", "socks5h://all:1080")]);
        assert_eq!(
            only_all.for_target(Some("https://ci")),
            proxy("ALL_PROXY", "socks5h://all:1080")
        );
    }

    #[test]
    fn lowercase_wins() {
        let env = proxy_env(&[
            ("HTTPS_PROXY", "http://upper:1"),
            ("https_proxy", "http://lower:1"),
        ]);
        assert_eq!(
            env.for_target(Some("https://ci")),
            proxy("https_proxy", "http://lower:1")
        );
    }

    #[test]
    fn unknown_target_assumes_https() {
        let env = proxy_env(&[("HTTPS_PROXY", "http://secure:3128")]);
        assert_eq!(
            env.for_target(None),
            proxy("HTTPS_PROXY", "http://secure:3128")
        );
    }

    #[test]
    fn no_proxy_bypasses_matching_hosts() {
        let env = proxy_env(&[
            ("HTTPS_PROXY", "http://secure:3128"),
            ("NO_PROXY", "localhost, .internal.corp,example.org"),
        ]);
        let bypassed = SystemProxy::Bypassed { var: "NO_PROXY" };
        assert_eq!(env.for_target(Some("https://localhost:8080")), bypassed);
        assert_eq!(env.for_target(Some("https://ci.internal.corp")), bypassed);
        assert_eq!(env.for_target(Some("https://internal.corp")), bypassed);
        assert_eq!(env.for_target(Some("https://EXAMPLE.org")), bypassed);
        assert_eq!(
            env.for_target(Some("https://notexample.org")),
            proxy("HTTPS_PROXY", "http://secure:3128")
        );
    }

    #[test]
    fn no_proxy_star_matches_everything() {
        let env = proxy_env(&[("ALL_PROXY", "socks5://p:1"), ("no_proxy", "*")]);
        assert_eq!(
            env.for_target(Some("https://ci")),
            SystemProxy::Bypassed { var: "no_proxy" }
        );
    }
}
