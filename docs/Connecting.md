Leeroy connects on startup and reconnects whenever a setting that affects the
connection changes (URL, headers, TLS, proxy). The header shows the state:
connecting, connected (Jenkins version and user), or the reason it failed.

## Authentication

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
- With `RUST_LOG=debug` (see [Troubleshooting](Troubleshooting.md)), the log
  shows which header *names* were sent, the final URL and the response status
  (never header values).

Env-provided headers win over the file and are read-only in the TUI.
Credentials embedded in the Jenkins URL (`https://user:pass@host`) also work, but
end up in the config file; they are masked wherever the URL is shown, except
while you edit it.

## Proxy

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
