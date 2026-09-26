//! Settings: where the config file lives, loading/saving it, and env var
//! overrides.
//!
//! Precedence for the file location: `--config` > `$LEEROY_CONFIG` >
//! `$XDG_CONFIG_HOME/leeroy/config.toml` (default `~/.config/...`). The legacy
//! `~/.leeroy/config.toml` is only used if it exists and the XDG file doesn't.
//! Precedence for each setting: its env var (see [`SettingKey::env_var`]) >
//! the config file.
//!
//! Functions take the environment (`env`) and filesystem checks (`exists`) as
//! parameters instead of reading them directly, so they can be tested
//! deterministically.

use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Result, WrapErr};
use serde::Deserialize;
use toml_edit::DocumentMut;

pub const CONFIG_ENV: &str = "LEEROY_CONFIG";
pub const FILE_NAME: &str = "config.toml";

/// Every user-facing setting. The TUI settings view lists them in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKey {
    JenkinsUrl,
    JenkinsUsername,
    JenkinsToken,
}

impl SettingKey {
    pub const ALL: [SettingKey; 3] = [
        SettingKey::JenkinsUrl,
        SettingKey::JenkinsUsername,
        SettingKey::JenkinsToken,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingKey::JenkinsUrl => "Jenkins URL",
            SettingKey::JenkinsUsername => "Username",
            SettingKey::JenkinsToken => "API token",
        }
    }

    /// `(table, key)` in the TOML file.
    pub fn toml_path(self) -> (&'static str, &'static str) {
        match self {
            SettingKey::JenkinsUrl => ("jenkins", "url"),
            SettingKey::JenkinsUsername => ("jenkins", "username"),
            SettingKey::JenkinsToken => ("jenkins", "token"),
        }
    }

    /// `LEEROY_<TABLE>_<KEY>`, uppercased.
    pub fn env_var(self) -> &'static str {
        match self {
            SettingKey::JenkinsUrl => "LEEROY_JENKINS_URL",
            SettingKey::JenkinsUsername => "LEEROY_JENKINS_USERNAME",
            SettingKey::JenkinsToken => "LEEROY_JENKINS_TOKEN",
        }
    }

    /// Secrets are masked in the UI.
    pub fn is_secret(self) -> bool {
        matches!(self, SettingKey::JenkinsToken)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub jenkins: JenkinsSettings,
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct JenkinsSettings {
    pub url: Option<String>,
    pub username: Option<String>,
    pub token: Option<String>,
}

/// Hand-written so the token never ends up in logs or panic messages.
impl std::fmt::Debug for JenkinsSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JenkinsSettings")
            .field("url", &self.url)
            .field("username", &self.username)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl Settings {
    pub fn get(&self, key: SettingKey) -> Option<&str> {
        match key {
            SettingKey::JenkinsUrl => self.jenkins.url.as_deref(),
            SettingKey::JenkinsUsername => self.jenkins.username.as_deref(),
            SettingKey::JenkinsToken => self.jenkins.token.as_deref(),
        }
    }

    pub fn set(&mut self, key: SettingKey, value: Option<String>) {
        let slot = match key {
            SettingKey::JenkinsUrl => &mut self.jenkins.url,
            SettingKey::JenkinsUsername => &mut self.jenkins.username,
            SettingKey::JenkinsToken => &mut self.jenkins.token,
        };
        *slot = value;
    }

    /// Settings given through env vars. Empty or non-UTF-8 values count as unset.
    pub fn from_env(env: impl Fn(&str) -> Option<OsString>) -> Self {
        let mut settings = Settings::default();
        for key in SettingKey::ALL {
            settings.set(key, non_empty(env(key.env_var())));
        }
        settings
    }

    /// `self` with every value set in `overrides` taking precedence.
    pub fn overlaid(&self, overrides: &Settings) -> Settings {
        let mut merged = self.clone();
        for key in SettingKey::ALL {
            if let Some(value) = overrides.get(key) {
                merged.set(key, Some(value.to_owned()));
            }
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
    toml::from_str(&text).wrap_err_with(|| format!("parsing {}", path.display()))
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
                doc[table][name] = toml_edit::value(value);
            }
            None => {
                if let Some(table) = doc.get_mut(table).and_then(|t| t.as_table_like_mut()) {
                    table.remove(name);
                }
            }
        }
    }

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
        file.set(SettingKey::JenkinsUsername, Some("file-user".into()));
        let overrides = Settings::from_env(env(&[
            ("LEEROY_JENKINS_URL", "https://env"),
            ("LEEROY_JENKINS_USERNAME", ""),
        ]));

        let merged = file.overlaid(&overrides);
        assert_eq!(merged.get(SettingKey::JenkinsUrl), Some("https://env"));
        assert_eq!(merged.get(SettingKey::JenkinsUsername), Some("file-user"));
        assert_eq!(merged.get(SettingKey::JenkinsToken), None);
    }

    #[test]
    fn debug_redacts_token() {
        let mut settings = Settings::default();
        settings.set(SettingKey::JenkinsToken, Some("s3cret".into()));
        let debug = format!("{settings:?}");
        assert!(!debug.contains("s3cret"), "{debug}");
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
        settings.set(SettingKey::JenkinsToken, Some("s3cret".into()));

        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), settings);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("[jenkins]\n"), "{text}");
    }

    #[test]
    fn save_keeps_comments_and_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            "# my notes\n[jenkins]\nurl = \"https://old\" # prod\nusername = \"gone\"\nfuture = 1\n",
        )
        .unwrap();
        let mut settings = load(&path).unwrap();
        settings.set(SettingKey::JenkinsUrl, Some("https://new".into()));
        settings.set(SettingKey::JenkinsUsername, None);

        save(&path, &settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my notes"), "{text}");
        assert!(text.contains("future = 1"), "{text}");
        assert!(text.contains("url = \"https://new\""), "{text}");
        assert!(!text.contains("username"), "{text}");
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
