//! Reading and storing secrets: the OS keyring, a password command and the
//! tokens file. IO; the executor in `main.rs` calls these from spawned tasks.
//! Errors are short, user-facing strings that never contain the secret.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use crate::auth::Secret;

/// The keyring service name all of Leeroy's entries are filed under.
pub const KEYRING_SERVICE: &str = "leeroy";

/// How long a password command may take (e.g. waiting for a GUI pinentry).
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

/// The keyring entry `account`, if set.
pub async fn keyring_get(account: &str) -> Result<Option<Secret>, String> {
    let account = account.to_owned();
    blocking(move || keyring::get(&account)).await
}

/// Store `secret` as keyring entry `account`.
pub async fn keyring_set(account: &str, secret: &Secret) -> Result<(), String> {
    let (account, secret) = (account.to_owned(), secret.clone());
    blocking(move || keyring::set(&account, &secret)).await
}

/// Remove keyring entry `account` (fine if it doesn't exist).
pub async fn keyring_delete(account: &str) -> Result<(), String> {
    let account = account.to_owned();
    blocking(move || keyring::delete(&account)).await
}

/// Keyring calls block (D-Bus, Keychain prompts): keep them off the runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|err| format!("keyring task failed: {err}"))?
}

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "macos",
    windows
))]
mod keyring {
    use super::{KEYRING_SERVICE, Secret};

    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, account).map_err(describe)
    }

    pub fn get(account: &str) -> Result<Option<Secret>, String> {
        match entry(account)?.get_password() {
            Ok(secret) => Ok(Some(Secret::new(secret))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(describe(err)),
        }
    }

    pub fn set(account: &str, secret: &Secret) -> Result<(), String> {
        entry(account)?
            .set_password(secret.expose())
            .map_err(describe)
    }

    pub fn delete(account: &str) -> Result<(), String> {
        match entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(describe(err)),
        }
    }

    /// Never includes the secret (`BadEncoding` carries its bytes).
    fn describe(err: keyring::Error) -> String {
        match err {
            keyring::Error::PlatformFailure(err) | keyring::Error::NoStorageAccess(err) => {
                format!("{}: {err}", super::NO_KEYRING)
            }
            keyring::Error::BadEncoding(_) => "the keyring entry isn't valid UTF-8".into(),
            keyring::Error::TooLong(what, max) => format!(
                "too long for the keyring ({what}, at most {max}): use the file store instead"
            ),
            other => format!("keyring: {other}"),
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "macos",
    windows
)))]
mod keyring {
    use super::Secret;

    pub fn get(_: &str) -> Result<Option<Secret>, String> {
        Err(super::NO_KEYRING.into())
    }

    pub fn set(_: &str, _: &Secret) -> Result<(), String> {
        Err(super::NO_KEYRING.into())
    }

    pub fn delete(_: &str) -> Result<(), String> {
        Err(super::NO_KEYRING.into())
    }
}

const NO_KEYRING: &str = "no OS keyring available (choose the command or file store)";

/// Run `command` through the shell and return the first line it prints.
/// It gets no terminal (stdin is closed, output captured): the TUI owns it.
pub async fn password_from_command(command: &str, timeout: Duration) -> Result<Secret, String> {
    let mut cmd = if cfg!(windows) {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| {
            format!(
                "the password command didn't finish within {}s",
                timeout.as_secs()
            )
        })?
        .map_err(|err| format!("couldn't run the password command: {err}"))?;
    if !output.status.success() {
        // Its stderr may echo the secret; only its length goes to the log.
        tracing::debug!(
            stderr_bytes = output.stderr.len(),
            "password command failed"
        );
        return Err(match output.status.code() {
            Some(code) => format!("the password command failed (exit code {code})"),
            None => "the password command was killed".into(),
        });
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "the password command printed invalid UTF-8".to_owned())?;
    let first = stdout.lines().next().unwrap_or("");
    // Spaces around it are a copy-paste accident, never part of a token.
    let line = first.trim();
    if line.len() != first.len() {
        tracing::debug!("password command: removed whitespace around its first line");
    }
    if line.is_empty() {
        return Err("the password command printed nothing".into());
    }
    Ok(Secret::new(line))
}

/// The tokens file entry `account`, if any (read off the async runtime).
pub async fn tokens_file_get(path: &Path, account: &str) -> Result<Option<Secret>, String> {
    let (path, account) = (path.to_owned(), account.to_owned());
    blocking(move || read_entry(&path, &account)).await
}

/// Store (`None`: remove) tokens file entry `account` (written and synced
/// off the async runtime).
pub async fn tokens_file_set(
    path: &Path,
    account: &str,
    secret: Option<&Secret>,
) -> Result<(), String> {
    let (path, account, secret) = (path.to_owned(), account.to_owned(), secret.cloned());
    blocking(move || write_entry(&path, &account, secret.as_ref())).await
}

/// The tokens file: `account = "<refresh token>"` lines, mode 0600.
pub fn tokens_path(state_dir: &Path) -> PathBuf {
    state_dir.join("tokens.toml")
}

fn read_entry(path: &Path, account: &str) -> Result<Option<Secret>, String> {
    Ok(read_tokens(path)?.remove(account).map(Secret::new))
}

/// Store (or with `None` remove) `account` in the tokens file.
fn write_entry(path: &Path, account: &str, secret: Option<&Secret>) -> Result<(), String> {
    let mut tokens = read_tokens(path)?;
    match secret {
        Some(secret) => tokens.insert(account.to_owned(), secret.expose().to_owned()),
        None => tokens.remove(account),
    };
    let text = toml::to_string(&tokens).map_err(|err| format!("encoding tokens: {err}"))?;
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let fail = |err: io::Error| format!("writing {}: {err}", path.display());
    fs::create_dir_all(dir).map_err(fail)?;
    // NamedTempFile is created with mode 0600 on Unix; rename = atomic.
    let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(fail)?;
    tmp.write_all(text.as_bytes()).map_err(fail)?;
    tmp.as_file().sync_all().map_err(fail)?;
    tmp.persist(path).map_err(|err| fail(err.error))?;
    Ok(())
}

fn read_tokens(path: &Path) -> Result<BTreeMap<String, String>, String> {
    match fs::read_to_string(path) {
        Ok(text) => {
            toml::from_str(&text).map_err(|err| format!("parsing {}: {err}", path.display()))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(err) => Err(format!("reading {}: {err}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = tokens_path(&dir.path().join("leeroy"));
        assert_eq!(read_entry(&path, "a"), Ok(None));
        write_entry(&path, "a", Some(&Secret::new("one"))).unwrap();
        write_entry(&path, "b", Some(&Secret::new("two"))).unwrap();
        assert_eq!(read_entry(&path, "a"), Ok(Some(Secret::new("one"))));
        write_entry(&path, "a", None).unwrap();
        assert_eq!(read_entry(&path, "a"), Ok(None));
        assert_eq!(read_entry(&path, "b"), Ok(Some(Secret::new("two"))));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_gives_its_first_line() {
        let secret = password_from_command("printf 'tok3n\\nother line\\n'", COMMAND_TIMEOUT).await;
        assert_eq!(secret, Ok(Secret::new("tok3n")));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn command_gives_its_first_line() {
        let secret = password_from_command("echo tok3n", COMMAND_TIMEOUT).await;
        assert_eq!(
            secret.map(|s| s.expose().trim().to_owned()),
            Ok("tok3n".into())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_output_is_trimmed() {
        let secret =
            password_from_command("printf ' tok3n \\t\\r\\nother\\n'", COMMAND_TIMEOUT).await;
        assert_eq!(secret, Ok(Secret::new("tok3n")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_failures_never_show_output() {
        let err = password_from_command("echo s3cret >&2; exit 3", COMMAND_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(err, "the password command failed (exit code 3)");
        let err = password_from_command("true", COMMAND_TIMEOUT)
            .await
            .unwrap_err();
        assert!(err.contains("printed nothing"), "{err}");
        let err = password_from_command("sleep 5", Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(err.contains("didn't finish"), "{err}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn command_gets_no_terminal() {
        // stdin is closed: a command reading it ends instead of hanging.
        let secret = password_from_command("cat; echo done", COMMAND_TIMEOUT).await;
        assert_eq!(secret, Ok(Secret::new("done")));
    }
}
