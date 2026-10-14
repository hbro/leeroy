Settings can be edited in the TUI (Settings tab: `0`) and are saved to a TOML file:

1. `--config FILE`, if given
2. else `$LEEROY_CONFIG`
3. else `$XDG_CONFIG_HOME/leeroy/config.toml` (`~/.config/leeroy/config.toml`
   when `$XDG_CONFIG_HOME` is unset)

An existing `~/.leeroy/config.toml` is still picked up when there's no file at
step 3; new configs are always created in the XDG location. If both exist,
Leeroy uses the XDG one and prints a warning to stderr.

On **Windows**, step 3 is `%XDG_CONFIG_HOME%\leeroy\config.toml` only when that
variable is set to an absolute path; otherwise the config lives in
`%APPDATA%\leeroy\config.toml`. The log file defaults to
`%LOCALAPPDATA%\leeroy\leeroy.log` there (`~/.local/state/leeroy/leeroy.log`
elsewhere; `LEEROY_LOG` overrides both).

```toml
[jenkins]
url = "https://jenkins.example.com"
skip_tls_verify = false  # default; see "TLS" below

[jenkins.headers]  # see Authentication on the Connecting page; the file is written with mode 0600
Authorization = "Basic bWU6czNjcmV0"

[proxy]
url = "socks5h://user:pass@bastion.example.com:1080"

[refresh]
auto = true    # default: start with auto-refresh on (toggle with R at any time)
interval = 10  # default; seconds between automatic refreshes, minimum 1

[ui]
confirm_quit = true  # default: ask before quitting with q
theme = "auto"       # default; or "dark" / "light", see "Colours" below
timestamps = "relative"  # default ("17m ago"); or "absolute" (local time), t switches
```

Every setting can be overridden by an env var, which wins over the file (and
can't be edited in the TUI while set):

| Setting  | Env var                   |
| -------- | ------------------------- |
| URL      | `LEEROY_JENKINS_URL`      |
| Header `<Name>` | `LEEROY_JENKINS_HEADERS_<NAME>` (`-` written as `_`, e.g. `LEEROY_JENKINS_HEADERS_X_API_KEY`) |
| Skip TLS verify | `LEEROY_JENKINS_SKIP_TLS_VERIFY` (`true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`) |
| Proxy    | `LEEROY_PROXY_URL`        |
| Auto-refresh | `LEEROY_REFRESH_AUTO` |
| Refresh interval | `LEEROY_REFRESH_INTERVAL` (seconds, ≥ 1) |
| Confirm quit | `LEEROY_UI_CONFIRM_QUIT` |
| Theme | `LEEROY_UI_THEME` (`auto`/`dark`/`light`) |
| Timestamps | `LEEROY_UI_TIMESTAMPS` (`relative`/`absolute`) |

Saving from the TUI keeps comments and unknown keys in the file.

Headers (authentication) and the proxy are explained under
[Connecting](Connecting.md).

## TLS

Certificates are checked against the operating system's trust store, so a
company CA installed system-wide just works. For self-signed or otherwise
invalid certificates, `skip_tls_verify = true` (toggle "Skip TLS verify" in the
settings) disables verification. **This is insecure**: anyone between you and
Jenkins can read your credentials. It's off by default, and while it's on the
header shows `⚠ TLS NOT VERIFIED`.

## Colours

Leeroy keeps your terminal's own background and colour scheme; the theme only
picks which of its colours go where. `dark` suits dark terminal backgrounds,
`light` light ones (inverted bars, a light selection, darker yellow and green).
The default, `auto`, asks the terminal for its background colour at startup and
picks accordingly; terminals that don't answer (tmux, for one) get `dark`. The
settings view shows what was detected. Change it with Enter on "Theme" in the
settings (it applies right away), `ui.theme` in the file, or `LEEROY_UI_THEME`.
