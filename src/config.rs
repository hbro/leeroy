//! Settings: where the config file lives, loading/saving it, and env var
//! overrides.
//!
//! Precedence for the file location: `--config` > `$LEEROY_CONFIG` >
//! `$XDG_CONFIG_HOME/leeroy/config.toml` (default `~/.config/...`). The legacy
//! `~/.leeroy/config.toml` is only used if it exists and the XDG file doesn't.
//! Precedence for each setting: its env var (see [`SettingKey::env_var`]) >
//! the config file. Custom HTTP headers (`[jenkins.headers]`) work the same way,
//! per header: `LEEROY_JENKINS_HEADERS_<NAME>` (see [`header_env_var`]).
//!
//! Functions take the environment (`env`) and filesystem checks (`exists`) as
//! parameters instead of reading them directly, so they can be tested
//! deterministically.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, WrapErr, bail, eyre};
use serde::Deserialize;
use toml_edit::DocumentMut;

pub const CONFIG_ENV: &str = "LEEROY_CONFIG";
pub const FILE_NAME: &str = "config.toml";

/// Every user-facing setting. The TUI settings view lists them in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    JenkinsUrl,
    JenkinsSkipTlsVerify,
    ProxyUrl,
}

impl SettingKey {
    pub const ALL: [SettingKey; 3] = [
        SettingKey::JenkinsUrl,
        SettingKey::JenkinsSkipTlsVerify,
        SettingKey::ProxyUrl,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingKey::JenkinsUrl => "Jenkins URL",
            SettingKey::JenkinsSkipTlsVerify => "Skip TLS verify",
            SettingKey::ProxyUrl => "Proxy URL",
        }
    }

    /// `(table, key)` in the TOML file.
    pub fn toml_path(self) -> (&'static str, &'static str) {
        match self {
            SettingKey::JenkinsUrl => ("jenkins", "url"),
            SettingKey::JenkinsSkipTlsVerify => ("jenkins", "skip_tls_verify"),
            SettingKey::ProxyUrl => ("proxy", "url"),
        }
    }

    /// `LEEROY_<TABLE>_<KEY>`, uppercased.
    pub fn env_var(self) -> &'static str {
        match self {
            SettingKey::JenkinsUrl => "LEEROY_JENKINS_URL",
            SettingKey::JenkinsSkipTlsVerify => "LEEROY_JENKINS_SKIP_TLS_VERIFY",
            SettingKey::ProxyUrl => "LEEROY_PROXY_URL",
        }
    }

    /// On/off settings: toggled instead of edited, stored as TOML booleans,
    /// and exposed through [`Settings::get`] as `"true"` / `"false"`.
    pub fn is_bool(self) -> bool {
        matches!(self, SettingKey::JenkinsSkipTlsVerify)
    }

    /// User-facing documentation, one line per `\n`. Shown in the settings
    /// view and written as a comment above the key when it's first saved.
    pub fn doc(self) -> &'static str {
        match self {
            SettingKey::JenkinsUrl => concat!(
                "Base URL of the Jenkins instance, e.g. https://jenkins.example.com\n",
                "For authentication, add headers below rather than user:pass@ in the URL.",
            ),
            SettingKey::JenkinsSkipTlsVerify => concat!(
                "Accept invalid TLS certificates (self-signed, expired, unknown CA) from Jenkins.\n",
                "INSECURE: anyone between you and Jenkins can read your credentials. Off by default;\n",
                "prefer adding the internal CA to the system trust store, which Leeroy uses.",
            ),
            SettingKey::ProxyUrl => concat!(
                "Proxy for reaching Jenkins: scheme://[user:pass@]host:port\n",
                "Schemes: http, https, socks4, socks4a, socks5, socks5h.\n",
                "SOCKS: socks5h:// or socks4a:// resolve DNS on the proxy (remote DNS);\n",
                "socks5:// and socks4:// resolve DNS locally. HTTP(S) proxies always resolve remotely.\n",
                "Unset: HTTPS_PROXY / HTTP_PROXY / ALL_PROXY / NO_PROXY are used.",
            ),
        }
    }

    /// Check (and normalize) a value before it's stored.
    pub fn validate(self, value: &str) -> Result<String, String> {
        let value = value.trim();
        match self {
            SettingKey::JenkinsUrl => {
                let url = parse_url(value)?;
                if !matches!(url.scheme(), "http" | "https") {
                    return Err(format!(
                        "unsupported scheme {:?}: use http or https",
                        url.scheme()
                    ));
                }
                Ok(value.to_owned())
            }
            SettingKey::ProxyUrl => {
                let url = parse_url(value)?;
                if !PROXY_SCHEMES.contains(&url.scheme()) {
                    return Err(format!(
                        "unsupported scheme {:?}: use one of {}",
                        url.scheme(),
                        PROXY_SCHEMES.join(", ")
                    ));
                }
                Ok(value.to_owned())
            }
            SettingKey::JenkinsSkipTlsVerify => parse_bool(value)
                .map(|b| b.to_string())
                .ok_or_else(|| format!("expected true or false, got {value:?}")),
        }
    }

    /// The value as it may be shown on screen or in logs: credentials in
    /// URLs are masked.
    pub fn display(self, value: &str) -> String {
        match self {
            SettingKey::JenkinsUrl | SettingKey::ProxyUrl => redact_url(value),
            _ if self.is_bool() => match parse_bool(value) {
                Some(true) => "on".into(),
                _ => "off".into(),
            },
            _ => value.to_owned(),
        }
    }
}

/// Documentation for `[jenkins.headers]`: shown in the settings view and
/// written as a comment above the table when it's first created.
pub const HEADERS_DOC: &str = concat!(
    "Extra HTTP headers sent with every request to Jenkins, e.g. for authentication.\n",
    "Edit as \"Name: value\"; clear the field to remove it. Values are masked, except while editing.\n",
    "Examples:\n",
    "  Authorization: Basic <base64 of user:password>\n",
    "  Authorization: Bearer <token>\n",
    "  X-Forwarded-User: <user>\n",
    "Basic auth covers reverse-proxy logins and a Jenkins user + API token alike.\n",
    "Base64: printf '%s' 'user:password' | base64 -w0\n",
    "From a password manager, via env var (read-only here, never saved):\n",
    "  LEEROY_JENKINS_HEADERS_AUTHORIZATION=\"Basic $(printf 'me:%s' \"$(pass show jenkins | head -n1)\" | base64 -w0)\"",
);

const HEADERS_ENV_PREFIX: &str = "LEEROY_JENKINS_HEADERS_";

/// The env var for header `name`: `LEEROY_JENKINS_HEADERS_<NAME>`, with `-`
/// written as `_`.
pub fn header_env_var(name: &str) -> String {
    format!(
        "{HEADERS_ENV_PREFIX}{}",
        name.to_ascii_uppercase().replace('-', "_")
    )
}

/// Header name from the env var suffix: `X_API_KEY` -> `X-Api-Key`.
fn header_name_from_env(suffix: &str) -> String {
    suffix
        .split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Parse `Name: value` into a valid HTTP header name and value.
pub fn parse_header(line: &str) -> Result<(String, String), String> {
    let (name, value) = line
        .split_once(':')
        .ok_or_else(|| "expected \"Name: value\"".to_owned())?;
    let (name, value) = (name.trim(), value.trim());
    validate_header(name, value)?;
    Ok((name.to_owned(), value.to_owned()))
}

fn validate_header(name: &str, value: &str) -> Result<(), String> {
    reqwest::header::HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| format!("invalid header name {name:?}"))?;
    if value.is_empty() {
        return Err(format!(
            "header {name} has no value (clear the whole field to remove it)"
        ));
    }
    reqwest::header::HeaderValue::from_str(value)
        .map_err(|_| format!("invalid value for header {name}"))?;
    if name.eq_ignore_ascii_case("authorization") {
        check_basic_auth(value)?;
    }
    Ok(())
}

/// Catch the usual mistakes in hand-made `Basic <base64>` values, which the
/// server would otherwise just answer with an unhelpful 401.
fn check_basic_auth(value: &str) -> Result<(), String> {
    use base64::Engine as _;
    let Some((scheme, encoded)) = value.split_once(' ') else {
        return Ok(());
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return Ok(());
    }
    let hint = "create it with: printf '%s' 'user:password' | base64 -w0";
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| format!("Authorization: Basic value isn't valid base64 ({hint})"))?;
    let decoded = String::from_utf8_lossy(&decoded);
    if decoded.contains(['\n', '\r']) {
        return Err(format!(
            "Authorization: Basic credentials contain a newline, e.g. from echo or a \
             multi-line `pass show` entry (use `pass show NAME | head -n1`; {hint})"
        ));
    }
    if !decoded.contains(':') {
        return Err(format!(
            "Authorization: Basic credentials must be user:password ({hint})"
        ));
    }
    Ok(())
}

/// Shown when an old config still uses the removed basic-auth settings.
const LEGACY_AUTH_HELP: &str = "\
jenkins.username / jenkins.token (and $LEEROY_JENKINS_USERNAME / $LEEROY_JENKINS_TOKEN) \
were replaced by custom headers. For HTTP basic auth, remove them and add:
  [jenkins.headers]
  Authorization = \"Basic <base64 of user:token>\"
(create the value with: printf '%s' 'user:token' | base64 -w0), or set
$LEEROY_JENKINS_HEADERS_AUTHORIZATION instead.";

/// `true`/`false`, plus the usual env var spellings (`1`/`0`, `yes`/`no`, `on`/`off`).
pub fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub const PROXY_SCHEMES: &[&str] = &["http", "https", "socks4", "socks4a", "socks5", "socks5h"];

fn parse_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|err| format!("invalid URL: {err}"))?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err("invalid URL: missing host".into());
    }
    Ok(url)
}

/// Byte range of the password in `scheme://user:password@host...`, if any.
fn password_range(value: &str) -> Option<(usize, usize)> {
    let authority_start = value.find("://")? + 3;
    let rest = &value[authority_start..];
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let at = authority.rfind('@')?;
    let colon = authority[..at].find(':')?;
    Some((authority_start + colon + 1, authority_start + at))
}

/// Mask the password of a URL (`user:pass@` -> `user:••••@`). String-based,
/// so the rest of the value is shown exactly as typed; input without a
/// password (or not yet a full URL) is returned unchanged.
pub fn redact_url(value: &str) -> String {
    match password_range(value) {
        Some((start, end)) => format!("{}••••{}", &value[..start], &value[end..]),
        None => value.to_owned(),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub jenkins: JenkinsSettings,
    pub proxy: ProxySettings,
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    pub url: Option<String>,
}

/// Hand-written so proxy credentials never end up in logs.
impl std::fmt::Debug for ProxySettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxySettings")
            .field("url", &self.url.as_deref().map(redact_url))
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct JenkinsSettings {
    pub url: Option<String>,
    pub skip_tls_verify: Option<bool>,
    /// Extra HTTP headers, name as written -> value. Names are compared
    /// case-insensitively (see [`Settings::header`]).
    pub headers: BTreeMap<String, String>,
}

/// Hand-written so header values and URL credentials never end up in logs.
impl std::fmt::Debug for JenkinsSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JenkinsSettings")
            .field("url", &self.url.as_deref().map(redact_url))
            .field("skip_tls_verify", &self.skip_tls_verify)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Settings {
    pub fn get(&self, key: SettingKey) -> Option<&str> {
        match key {
            SettingKey::JenkinsUrl => self.jenkins.url.as_deref(),
            SettingKey::JenkinsSkipTlsVerify => self
                .jenkins
                .skip_tls_verify
                .map(|b| if b { "true" } else { "false" }),
            SettingKey::ProxyUrl => self.proxy.url.as_deref(),
        }
    }

    /// Store a value. It must have passed [`SettingKey::validate`]; for
    /// boolean keys, an unparsable value unsets the setting.
    pub fn set(&mut self, key: SettingKey, value: Option<String>) {
        let slot = match key {
            SettingKey::JenkinsUrl => &mut self.jenkins.url,
            SettingKey::JenkinsSkipTlsVerify => {
                self.jenkins.skip_tls_verify = value.as_deref().and_then(parse_bool);
                return;
            }
            SettingKey::ProxyUrl => &mut self.proxy.url,
        };
        *slot = value;
    }

    /// The header named `name` (case-insensitive): `(name as stored, value)`.
    pub fn header(&self, name: &str) -> Option<(&str, &str)> {
        self.jenkins
            .headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// Add, replace (case-insensitively) or, with `None`, remove a header.
    pub fn set_header(&mut self, name: &str, value: Option<String>) {
        self.jenkins
            .headers
            .retain(|n, _| !n.eq_ignore_ascii_case(name));
        if let Some(value) = value {
            self.jenkins.headers.insert(name.to_owned(), value);
        }
    }

    /// Settings whose value is on (for boolean keys).
    pub fn is_on(&self, key: SettingKey) -> bool {
        self.get(key).and_then(parse_bool).unwrap_or(false)
    }

    /// Settings given through env vars (all of them, e.g. `std::env::vars_os()`).
    /// Empty or non-UTF-8 values count as unset; invalid values are an error
    /// naming the variable.
    pub fn from_env(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Result<Self> {
        let vars: BTreeMap<String, String> = vars
            .into_iter()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, non_empty(Some(v))?)))
            .collect();
        if ["LEEROY_JENKINS_USERNAME", "LEEROY_JENKINS_TOKEN"]
            .iter()
            .any(|name| vars.contains_key(*name))
        {
            bail!("{LEGACY_AUTH_HELP}");
        }
        let mut settings = Settings::default();
        for key in SettingKey::ALL {
            if let Some(raw) = vars.get(key.env_var()) {
                let value = key
                    .validate(raw)
                    .map_err(|err| eyre!("invalid ${}: {err}", key.env_var()))?;
                settings.set(key, Some(value));
            }
        }
        for (var, value) in &vars {
            if let Some(suffix) = var.strip_prefix(HEADERS_ENV_PREFIX) {
                let name = header_name_from_env(suffix);
                validate_header(&name, value.trim())
                    .map_err(|err| eyre!("invalid ${var}: {err}"))?;
                settings.set_header(&name, Some(value.trim().to_owned()));
            }
        }
        Ok(settings)
    }

    /// Check every set value; `source` names where they came from in errors.
    pub fn validate(&self, source: impl Fn(SettingKey) -> String) -> Result<()> {
        for key in SettingKey::ALL {
            if let Some(value) = self.get(key) {
                key.validate(value)
                    .map_err(|err| eyre!("invalid {}: {err}", source(key)))?;
            }
        }
        for (name, value) in &self.jenkins.headers {
            validate_header(name, value).map_err(|err| eyre!("invalid jenkins.headers: {err}"))?;
        }
        Ok(())
    }

    /// `self` with every value set in `overrides` taking precedence.
    pub fn overlaid(&self, overrides: &Settings) -> Settings {
        let mut merged = self.clone();
        for key in SettingKey::ALL {
            if let Some(value) = overrides.get(key) {
                merged.set(key, Some(value.to_owned()));
            }
        }
        for (name, value) in &overrides.jenkins.headers {
            merged.set_header(name, Some(value.clone()));
        }
        merged
    }
}

fn non_empty(value: Option<OsString>) -> Option<String> {
    value
        .and_then(|v| v.into_string().ok())
        .filter(|v| !v.is_empty())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocation {
    pub path: PathBuf,
    /// A legacy `~/.leeroy/config.toml` that exists but is ignored because the
    /// XDG file exists too. Worth a warning: edits to it have no effect.
    pub ignored: Option<PathBuf>,
}

impl ConfigLocation {
    fn at(path: PathBuf) -> Self {
        Self {
            path,
            ignored: None,
        }
    }
}

/// Where the config file lives. `None` if no location can be determined
/// (no `$HOME`), in which case the caller should ask for `--config`.
///
/// `exists` reports whether a file exists (normally `Path::exists`).
pub fn resolve_path(
    cli: Option<PathBuf>,
    env: impl Fn(&str) -> Option<OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Option<ConfigLocation> {
    if let Some(path) = cli {
        return Some(ConfigLocation::at(path));
    }
    if let Some(path) = non_empty(env(CONFIG_ENV)) {
        return Some(ConfigLocation::at(path.into()));
    }
    let home = non_empty(env("HOME")).map(PathBuf::from);
    // Per the XDG spec: relative $XDG_CONFIG_HOME is ignored, default ~/.config.
    let xdg_dir = non_empty(env("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let xdg = xdg_dir.map(|dir| dir.join("leeroy").join(FILE_NAME));
    let legacy = home.map(|h| h.join(".leeroy").join(FILE_NAME));

    let legacy = legacy.filter(|legacy| exists(legacy));
    match (xdg, legacy) {
        (Some(xdg), Some(legacy)) if exists(&xdg) => Some(ConfigLocation {
            path: xdg,
            ignored: Some(legacy),
        }),
        (_, Some(legacy)) => Some(ConfigLocation::at(legacy)),
        (xdg, None) => xdg.map(ConfigLocation::at),
    }
}

/// Load the config file. A missing file means default settings.
pub fn load(path: &Path) -> Result<Settings> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(err) => return Err(err).wrap_err_with(|| format!("reading {}", path.display())),
    };
    let table: toml::Table =
        toml::from_str(&text).wrap_err_with(|| format!("parsing {}", path.display()))?;
    let jenkins = table.get("jenkins").and_then(|j| j.as_table());
    if jenkins.is_some_and(|j| j.contains_key("username") || j.contains_key("token")) {
        bail!("{}: {LEGACY_AUTH_HELP}", path.display());
    }
    toml::Value::Table(table)
        .try_into()
        .wrap_err_with(|| format!("parsing {}", path.display()))
}

/// Write `settings` to the config file.
///
/// Edits the existing document in place, so comments and keys Leeroy doesn't
/// know about survive. The write is atomic (temp file + rename) and the file
/// is created with mode 0600, since it may hold an API token. A symlinked
/// config file is followed, not replaced.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    let existing = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).wrap_err_with(|| format!("reading {}", target.display())),
    };
    let mut doc: DocumentMut = existing
        .parse()
        .wrap_err_with(|| format!("parsing {}", target.display()))?;

    for key in SettingKey::ALL {
        let (table, name) = key.toml_path();
        match settings.get(key) {
            Some(value) => {
                // Indexing would create an inline table; use a `[table]` section.
                if !doc.contains_key(table) {
                    doc[table] = toml_edit::table();
                }
                let is_new = doc[table].get(name).is_none();
                doc[table][name] = if key.is_bool() {
                    toml_edit::value(parse_bool(value).unwrap_or(false))
                } else {
                    toml_edit::value(value)
                };
                // Document a key the first time it's written; never touch
                // comments on existing keys.
                if is_new
                    && let Some(mut key_mut) =
                        doc[table].as_table_mut().and_then(|t| t.key_mut(name))
                {
                    let comment: String = key
                        .doc()
                        .lines()
                        .map(|line| format!("# {line}\n"))
                        .collect();
                    key_mut.leaf_decor_mut().set_prefix(comment);
                }
            }
            None => {
                if let Some(table) = doc.get_mut(table).and_then(|t| t.as_table_like_mut()) {
                    table.remove(name);
                }
            }
        }
    }

    save_headers(&mut doc, &settings.jenkins.headers);

    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
    // NamedTempFile is created with mode 0600 on Unix.
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .wrap_err_with(|| format!("creating temp file in {}", dir.display()))?;
    tmp.write_all(doc.to_string().as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(&target)
        .wrap_err_with(|| format!("writing {}", target.display()))?;
    Ok(())
}

/// Sync `[jenkins.headers]` with `headers`, keeping comments on existing
/// entries. Removed when empty.
fn save_headers(doc: &mut DocumentMut, headers: &BTreeMap<String, String>) {
    if headers.is_empty() {
        if let Some(jenkins) = doc.get_mut("jenkins").and_then(|t| t.as_table_like_mut()) {
            jenkins.remove("headers");
        }
        return;
    }
    if !doc.contains_key("jenkins") {
        doc["jenkins"] = toml_edit::table();
    }
    if doc["jenkins"].get("headers").is_none() {
        let mut table = toml_edit::Table::new();
        let comment: String = HEADERS_DOC
            .lines()
            .map(|line| format!("# {line}\n"))
            .collect();
        table.decor_mut().set_prefix(format!("\n{comment}"));
        doc["jenkins"]["headers"] = toml_edit::Item::Table(table);
    }
    let Some(table) = doc["jenkins"]["headers"].as_table_like_mut() else {
        return;
    };
    let stale: Vec<String> = table
        .iter()
        .map(|(name, _)| name.to_owned())
        .filter(|name| !headers.contains_key(name))
        .collect();
    for name in stale {
        table.remove(&name);
    }
    for (name, value) in headers {
        if table.get(name).and_then(|v| v.as_str()) != Some(value) {
            table.insert(name, toml_edit::value(value));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| vars.get(name).cloned()
    }

    /// All env vars, as `Settings::from_env` takes them.
    fn vars(vars: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        vars.iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    fn path_of(
        cli: Option<PathBuf>,
        env: impl Fn(&str) -> Option<OsString>,
        exists: impl Fn(&Path) -> bool,
    ) -> Option<PathBuf> {
        resolve_path(cli, env, exists).map(|l| l.path)
    }

    /// `exists` stub: only the given paths exist.
    fn files(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p| paths.iter().any(|x| x == p)
    }

    #[test]
    fn path_precedence() {
        let none = || files(&[]);
        let all = env(&[
            ("LEEROY_CONFIG", "/env/config.toml"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("HOME", "/home/u"),
        ]);
        assert_eq!(
            path_of(Some("/cli.toml".into()), &all, none()),
            Some("/cli.toml".into())
        );
        assert_eq!(path_of(None, &all, none()), Some("/env/config.toml".into()));

        let xdg = env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")]);
        assert_eq!(
            path_of(None, &xdg, none()),
            Some("/xdg/leeroy/config.toml".into())
        );

        // XDG default when $XDG_CONFIG_HOME is unset.
        let home = env(&[("HOME", "/home/u")]);
        assert_eq!(
            path_of(None, &home, none()),
            Some("/home/u/.config/leeroy/config.toml".into())
        );

        assert_eq!(path_of(None, env(&[]), none()), None);
    }

    #[test]
    fn legacy_path_only_when_it_exists_alone() {
        let home = env(&[("HOME", "/home/u")]);
        let legacy = "/home/u/.leeroy/config.toml";
        let xdg = "/home/u/.config/leeroy/config.toml";

        assert_eq!(path_of(None, &home, files(&[legacy])), Some(legacy.into()));
        assert_eq!(
            path_of(None, &home, files(&[legacy, xdg])),
            Some(xdg.into())
        );

        let explicit = env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/home/u")]);
        assert_eq!(
            path_of(None, &explicit, files(&[legacy])),
            Some(legacy.into())
        );
        assert_eq!(
            path_of(None, &explicit, files(&[legacy, "/xdg/leeroy/config.toml"])),
            Some("/xdg/leeroy/config.toml".into())
        );
    }

    #[test]
    fn ignored_legacy_is_reported() {
        let home = env(&[("HOME", "/home/u")]);
        let legacy = "/home/u/.leeroy/config.toml";
        let xdg = "/home/u/.config/leeroy/config.toml";

        let both = resolve_path(None, &home, files(&[legacy, xdg])).unwrap();
        assert_eq!(both.ignored, Some(legacy.into()));
        let only_legacy = resolve_path(None, &home, files(&[legacy])).unwrap();
        assert_eq!(only_legacy.ignored, None);
        // An explicit choice isn't second-guessed.
        let cli = resolve_path(Some("/cli.toml".into()), &home, files(&[legacy, xdg])).unwrap();
        assert_eq!(cli.ignored, None);
    }

    #[test]
    fn path_ignores_empty_and_relative_values() {
        let vars = env(&[
            ("LEEROY_CONFIG", ""),
            ("XDG_CONFIG_HOME", "relative/dir"),
            ("HOME", "/home/u"),
        ]);
        assert_eq!(
            path_of(None, vars, files(&[])),
            Some("/home/u/.config/leeroy/config.toml".into())
        );
    }

    #[test]
    fn env_var_names_follow_convention() {
        for key in SettingKey::ALL {
            let (table, name) = key.toml_path();
            let expected = format!("LEEROY_{table}_{name}").to_uppercase();
            assert_eq!(key.env_var(), expected);
        }
    }

    #[test]
    fn env_overrides_file() {
        let mut file = Settings::default();
        file.set(SettingKey::JenkinsUrl, Some("https://file".into()));
        file.set(SettingKey::ProxyUrl, Some("http://file-proxy:1".into()));
        let overrides = Settings::from_env(vars(&[
            ("LEEROY_JENKINS_URL", "https://env"),
            ("LEEROY_PROXY_URL", ""),
        ]))
        .unwrap();

        let merged = file.overlaid(&overrides);
        assert_eq!(merged.get(SettingKey::JenkinsUrl), Some("https://env"));
        assert_eq!(
            merged.get(SettingKey::ProxyUrl),
            Some("http://file-proxy:1")
        );
        assert_eq!(merged.get(SettingKey::JenkinsSkipTlsVerify), None);
    }

    #[test]
    fn headers_from_env_override_file_case_insensitively() {
        let mut file = Settings::default();
        file.set_header("authorization", Some("Bearer file".into()));
        file.set_header("X-Keep", Some("kept".into()));
        let env = Settings::from_env(vars(&[
            ("LEEROY_JENKINS_HEADERS_AUTHORIZATION", "Bearer env"),
            ("LEEROY_JENKINS_HEADERS_X_API_KEY", " k3y "),
            ("UNRELATED", "x"),
        ]))
        .unwrap();
        assert_eq!(env.header("X-Api-Key"), Some(("X-Api-Key", "k3y")));

        let merged = file.overlaid(&env);
        assert_eq!(
            merged.header("AUTHORIZATION"),
            Some(("Authorization", "Bearer env"))
        );
        assert_eq!(
            merged.jenkins.headers.len(),
            3,
            "{:?}",
            merged.jenkins.headers.keys()
        );
        assert_eq!(merged.header("x-keep"), Some(("X-Keep", "kept")));
    }

    #[test]
    fn header_env_var_names() {
        assert_eq!(
            header_env_var("X-Api-Key"),
            "LEEROY_JENKINS_HEADERS_X_API_KEY"
        );
        assert_eq!(header_name_from_env("X_FORWARDED_USER"), "X-Forwarded-User");
    }

    #[test]
    fn header_parsing() {
        assert_eq!(
            parse_header(" Authorization :  Basic bWU6czNjcmV0 "),
            Ok(("Authorization".into(), "Basic bWU6czNjcmV0".into()))
        );
        // Values may contain colons.
        assert_eq!(
            parse_header("X-Url: https://x"),
            Ok(("X-Url".into(), "https://x".into()))
        );
        assert!(parse_header("no colon").is_err());
        assert!(parse_header("Bad Name: x").is_err());
        assert!(parse_header("X-Empty:").is_err());
        assert!(Settings::from_env(vars(&[("LEEROY_JENKINS_HEADERS_BAD", "a\nb")])).is_err());
    }

    #[test]
    fn basic_auth_values_are_checked() {
        let ok = format!("Authorization: Basic {}", "bWU6czNjcmV0"); // me:s3cret
        assert!(parse_header(&ok).is_ok());
        assert!(parse_header("Authorization: Bearer anything").is_ok());
        // echo 'me:s3cret' | base64 (trailing newline)
        let err = parse_header("Authorization: Basic bWU6czNjcmV0Cg==").unwrap_err();
        assert!(err.contains("newline"), "{err}");
        // A multi-line pass entry: "me:pw\nuser: me"
        let err = parse_header("Authorization: Basic bWU6cHcKdXNlcjogbWU=").unwrap_err();
        assert!(err.contains("head -n1"), "{err}");
        assert!(
            parse_header("Authorization: Basic !!!")
                .unwrap_err()
                .contains("base64")
        );
        // base64("nocolon")
        assert!(
            parse_header("Authorization: Basic bm9jb2xvbg==")
                .unwrap_err()
                .contains("user:password")
        );
        // Also checked for env vars.
        assert!(
            Settings::from_env(vars(&[(
                "LEEROY_JENKINS_HEADERS_AUTHORIZATION",
                "Basic bWU6czNjcmV0Cg=="
            )]))
            .is_err()
        );
    }

    #[test]
    fn removed_auth_settings_fail_loudly() {
        let err = Settings::from_env(vars(&[("LEEROY_JENKINS_TOKEN", "t")])).unwrap_err();
        assert!(format!("{err}").contains("Authorization"), "{err}");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "[jenkins]\nusername = \"me\"\n").unwrap();
        let err = load(&path).unwrap_err();
        assert!(format!("{err}").contains("[jenkins.headers]"), "{err}");
    }

    #[test]
    fn debug_redacts_secrets() {
        let mut settings = Settings::default();
        settings.set(SettingKey::JenkinsUrl, Some("https://me:urlpass@ci".into()));
        settings.set_header("Authorization", Some("Basic s3cret".into()));
        let debug = format!("{settings:?}");
        assert!(
            !debug.contains("s3cret") && !debug.contains("urlpass"),
            "{debug}"
        );
        assert!(debug.contains("Authorization"), "{debug}");
    }

    #[test]
    fn proxy_url_validation() {
        let key = SettingKey::ProxyUrl;
        for ok in [
            "http://proxy:3128",
            "https://proxy:443",
            "socks4a://proxy:1080",
            "socks5h://user:pass@proxy:1080",
        ] {
            assert_eq!(key.validate(&format!(" {ok} ")), Ok(ok.to_owned()));
        }
        for bad in ["ftp://proxy:21", "proxy:3128", "socks5h://", "not a url"] {
            assert!(key.validate(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn jenkins_url_validation() {
        let key = SettingKey::JenkinsUrl;
        assert!(key.validate("https://ci.example.com").is_ok());
        assert!(key.validate("socks5://ci").is_err());
        assert!(key.validate("ci.example.com").is_err());
    }

    #[test]
    fn proxy_password_is_redacted() {
        let key = SettingKey::ProxyUrl;
        assert_eq!(
            key.display("socks5h://me:hunter2@proxy:1080"),
            "socks5h://me:••••@proxy:1080"
        );
        assert_eq!(key.display("http://me@proxy:1"), "http://me@proxy:1");
        assert_eq!(key.display("http://proxy:1/p@th"), "http://proxy:1/p@th");
        assert_eq!(key.display("http://me:hunt"), "http://me:hunt");

        let mut settings = Settings::default();
        settings.set(key, Some("http://me:hunter2@proxy:3128".into()));
        let debug = format!("{settings:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
    }

    #[test]
    fn invalid_values_fail_validation_with_source() {
        let mut settings = Settings::default();
        settings.set(SettingKey::ProxyUrl, Some("ftp://proxy".into()));
        let err = settings
            .validate(|key| format!("${}", key.env_var()))
            .unwrap_err();
        assert!(format!("{err}").contains("$LEEROY_PROXY_URL"), "{err}");
    }

    #[test]
    fn first_save_documents_key_but_keeps_user_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut settings = Settings::default();
        settings.set(SettingKey::ProxyUrl, Some("socks5h://proxy:1080".into()));
        save(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# SOCKS: socks5h://"), "{text}");
        assert_eq!(load(&path).unwrap(), settings);

        // Re-saving an existing key doesn't duplicate or replace comments.
        fs::write(&path, "[proxy]\n# mine\nurl = \"http://a:1\"\n").unwrap();
        settings.set(SettingKey::ProxyUrl, Some("http://b:1".into()));
        save(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text, "[proxy]\n# mine\nurl = \"http://b:1\"\n");
    }

    #[test]
    fn skip_tls_verify_is_a_bool() {
        let key = SettingKey::JenkinsSkipTlsVerify;
        assert!(!Settings::default().is_on(key), "must default to off");
        for (raw, on) in [("true", true), ("1", true), ("YES", true), ("off", false)] {
            let settings =
                Settings::from_env(vars(&[("LEEROY_JENKINS_SKIP_TLS_VERIFY", raw)])).unwrap();
            assert_eq!(settings.is_on(key), on, "{raw}");
        }
        let err =
            Settings::from_env(vars(&[("LEEROY_JENKINS_SKIP_TLS_VERIFY", "maybe")])).unwrap_err();
        assert!(
            format!("{err}").contains("$LEEROY_JENKINS_SKIP_TLS_VERIFY"),
            "{err}"
        );
    }

    #[test]
    fn skip_tls_verify_saved_as_toml_bool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut settings = Settings::default();
        settings.set(SettingKey::JenkinsSkipTlsVerify, Some("true".into()));
        save(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("skip_tls_verify = true\n"), "{text}");
        assert_eq!(load(&path).unwrap(), settings);

        // A string in the file is a parse error, not silently "off".
        fs::write(&path, "[jenkins]\nskip_tls_verify = \"yes\"\n").unwrap();
        assert!(load(&path).is_err());
    }

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&dir.path().join("nope.toml")).unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn invalid_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(&path, "jenkins = [").unwrap();
        let err = load(&path).unwrap_err();
        assert!(format!("{err}").contains("parsing"), "{err}");
    }

    #[test]
    fn save_roundtrip_creates_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b").join(FILE_NAME);
        let mut settings = Settings::default();
        settings.set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        settings.set_header("Authorization", Some("Basic s3cret".into()));

        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[jenkins]\n"), "{text}");
        assert!(text.contains("[jenkins.headers]\n"), "{text}");
        assert!(
            text.contains("# Examples:"),
            "headers table documented: {text}"
        );
        assert!(!text.contains("jenkins = {"), "{text}");

        // Removing the last header removes the table.
        settings.set_header("authorization", None);
        save(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("[jenkins.headers]"), "{text}");
        assert!(!text.contains("Authorization"), "{text}");
    }

    #[test]
    fn save_keeps_comments_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            "# my notes\n[jenkins]\nurl = \"https://old\" # prod\nskip_tls_verify = true\nfuture = 1\n",
        )
        .unwrap();
        let mut settings = load(&path).unwrap();
        settings.set(SettingKey::JenkinsUrl, Some("https://new".into()));
        settings.set(SettingKey::JenkinsSkipTlsVerify, None);

        save(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my notes"), "{text}");
        assert!(text.contains("future = 1"), "{text}");
        assert!(text.contains("url = \"https://new\""), "{text}");
        assert!(!text.contains("skip_tls_verify"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        save(&path, &Settings::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn save_follows_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles.toml");
        let link = dir.path().join(FILE_NAME);
        fs::write(&real, "").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let mut settings = Settings::default();
        settings.set(SettingKey::JenkinsUrl, Some("https://ci".into()));
        save(&link, &settings).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(load(&real).unwrap(), settings);
    }
}
