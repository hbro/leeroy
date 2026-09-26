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
```

Every setting can be overridden by an env var, which wins over the file (and
can't be edited in the TUI while set):

| Setting  | Env var                   |
| -------- | ------------------------- |
| URL      | `LEEROY_JENKINS_URL`      |
| Header `<Name>` | `LEEROY_JENKINS_HEADERS_<NAME>` (`-` written as `_`, e.g. `LEEROY_JENKINS_HEADERS_X_API_KEY`) |
| Skip TLS verify | `LEEROY_JENKINS_SKIP_TLS_VERIFY` (`true`/`false`, `1`/`0`, `yes`/`no`, `on`/`off`) |
| Proxy    | `LEEROY_PROXY_URL`        |

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
