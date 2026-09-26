# Leeroy
Jenkins TUI

## Configuration

Settings can be edited in the TUI (press `s`) and are saved to a TOML file:

1. `--config FILE`, if given
2. else `$LEEROY_CONFIG`
3. else `$XDG_CONFIG_HOME/leeroy/config.toml` (`~/.config/leeroy/config.toml`
   when `$XDG_CONFIG_HOME` is unset)

An existing `~/.leeroy/config.toml` is still picked up when there's no file at
step 3; new configs are always created in the XDG location. If both exist,
Leeroy uses the XDG one and prints a warning to stderr.

```toml
[jenkins]
url = "https://jenkins.example.com"
skip_tls_verify = false  # default; see "TLS" below

[jenkins.headers]  # see "Authentication" below; the file is written with mode 0600
Authorization = "Basic bWU6czNjcmV0"

[proxy]
url = "socks5h://user:pass@bastion.example.com:1080"

[refresh]
auto = true    # default: start with auto-refresh on (toggle with R at any time)
interval = 10  # default; seconds between automatic refreshes, minimum 1

[ui]
confirm_quit = true  # default: ask before quitting with q
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

Saving from the TUI keeps comments and unknown keys in the file.

### Proxy

When Jenkins is only reachable through a proxy, set `proxy.url` to
`scheme://[user:pass@]host:port`. The scheme selects the proxy type:

| Scheme       | Proxy type      | DNS resolved   |
| ------------ | --------------- | -------------- |
| `http`       | HTTP proxy      | on the proxy   |
| `https`      | HTTPS proxy     | on the proxy   |
| `socks4`     | SOCKS4          | locally        |
| `socks4a`    | SOCKS4a         | **on the proxy** |
| `socks5`     | SOCKS5          | locally        |
| `socks5h`    | SOCKS5          | **on the proxy** |

**Remote DNS with SOCKS:** use `socks5h://` (or `socks4a://`) when the Jenkins
host name only resolves inside the proxied network. With `socks5://` the name is
resolved on your machine first.

Without a proxy setting, the usual env vars apply, like with curl:
`https_proxy`/`HTTPS_PROXY` (or `http_proxy`/`HTTP_PROXY` for an `http://`
Jenkins URL), then `all_proxy`/`ALL_PROXY`, except for hosts listed in
`no_proxy`/`NO_PROXY` (host names or domain suffixes, or `*`). The settings
view shows which one is in effect. An explicit Leeroy proxy setting ignores
`NO_PROXY`.

Passwords in proxy URLs are masked on screen (except while you edit the field)
and in logs.

### Connecting

Leeroy connects on startup and reconnects whenever a setting that affects the
connection changes (URL, headers, TLS, proxy). The header shows the state:
connecting, connected (Jenkins version and user), or the reason it failed.

### Authentication

Leeroy doesn't assume an auth scheme: it sends whatever HTTP headers you
configure under `[jenkins.headers]`, with every request. Add them in the settings
view (`+ add header`, edit as `Name: value`, clear the field to remove one).
Header values are masked on screen (except in the field you're editing) and never
logged.

| Setup | Header |
| ----- | ------ |
| HTTP basic auth (reverse proxy in front of Jenkins) | `Authorization: Basic <base64 of user:password>` |
| Jenkins user + API token | `Authorization: Basic <base64 of user:api-token>` |
| Token-based SSO gateway | `Authorization: Bearer <token>` |
| Trusted reverse-proxy user header | `X-Forwarded-User: <user>` |

Create a basic-auth value with `printf '%s' 'user:password' | base64 -w0`.

To keep the secret out of the config file, pass the header as an env var, e.g.
from `pass`:

```sh
LEEROY_JENKINS_HEADERS_AUTHORIZATION="Basic $(printf 'me:%s' "$(pass show work/jenkins | head -n1)" | base64 -w0)" leeroy
```

(`head -n1`: `pass show` prints the whole entry, including any extra lines.)

**Troubleshooting a 401/403:** the error says who refused: Jenkins itself, or a
server in front of it (plus the login it asks for). Common causes:

- The credentials contain a newline (`echo … | base64` instead of `printf '%s' …`,
  or a multi-line `pass` entry). Leeroy rejects such `Basic` values up front.
- A redirect to another scheme/host/port (e.g. `http://` → `https://`): the
  `Authorization` header is dropped on such redirects for safety. The error names
  the redirect target; use that as the Jenkins URL.
- With `RUST_LOG=debug`, the log shows which header *names* were sent, the final
  URL and the response status (never header values).

Env-provided headers win over the file and are read-only in the TUI.
Credentials embedded in the Jenkins URL (`https://user:pass@host`) also work, but
end up in the config file; they are masked wherever the URL is shown, except
while you edit it.

The old `jenkins.username` / `jenkins.token` settings were replaced by headers;
Leeroy refuses to start while they are still set and explains the replacement.

### TLS

Certificates are checked against the operating system's trust store, so a
company CA installed system-wide just works. For self-signed or otherwise
invalid certificates, `skip_tls_verify = true` (toggle "Skip TLS verify" in the
settings) disables verification. **This is insecure**: anyone between you and
Jenkins can read your credentials. It's off by default, and while it's on the
header shows `⚠ TLS NOT VERIFIED`.

## Using Leeroy

Content lives in tabs, switched with the F-keys (shown in the tab bar):

| Key | Tab |
| --- | --- |
| `F1` | Jobs |

Global keys (bottom bar), available everywhere except while typing in a field:

| Key | Action |
| --- | --- |
| `q` | quit (asks first: `y`/`Enter`/`q` quits, `n`/`Esc` stays; setting `ui.confirm_quit`) |
| `Ctrl-C` | quit immediately, from anywhere |
| `s` | settings |
| `?` | help |
| `r` | refresh now (reconnects if the connection failed) |
| `R` | toggle auto-refresh for this session |

### Jobs

All jobs of the instance, with folders and multibranch projects flattened into
`folder/sub/job` names, their last result (colour *and* word) and `⟳` while a
build runs.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | move |
| `Enter` | details of the job's most recent build |
| `/` | filter (live, case-insensitive; space-separated words must all match) |
| `Enter` / `Esc` while filtering | apply / cancel |
| `Esc` | clear an applied filter |

The header's right end shows how long ago the data was fetched: `⟳ 4s`. The
icon is green while auto-refresh is on and gray while it's off; `⟳ …` means a
refresh is running and `✕` that the last one failed. The
default and the interval are in the settings (`[refresh]`); a failed refresh is
retried after a full interval, never in a tight loop.

### Last build

`Enter` on a job shows its most recent build: result, when it started, how long
it took, what triggered it, parameters, description and the SCM changes it
included. A running build shows its elapsed time against Jenkins' estimate as a
progress bar. `r` and auto-refresh re-fetch the build while this view is open (so
a running build updates live), and the header's `⟳` age refers to it.

| Key | Action |
| --- | --- |
| `←` / `→` | older / newer build (deleted builds are skipped) |
| `Home` / `End` | first (oldest kept) / latest build |
| `c` | console output of this build |
| `↑/↓` `PgUp/PgDn` `g/G` | scroll |
| `Esc` | back to the list |

The title shows which build you're on (`#42 · 17 of 20`, counted from the
oldest). On the latest build, refreshing follows new builds as they start; an
older build stays put.

### Console output

`c` in the build view opens the build's console output, scrolled to the end.
While the build runs, new output is fetched every second (only the new part)
and followed at the bottom. Scrolling up pauses following (the title says so);
`End` resumes it.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k` | one line |
| `PgUp/PgDn` | one page |
| `Home` / `End` | top / bottom (`End` = follow again) |
| `←/→` | scroll long lines sideways |
| `c` / `Esc` | back to the build (`c` toggles the console) |

Colour codes are removed, `\r` progress bars show their final state, and at
most the last 100 000 lines are kept.
