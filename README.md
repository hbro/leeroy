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
username = "me"
token = "..."   # API token; the file is written with mode 0600
skip_tls_verify = false  # default; see "TLS" below

[proxy]
url = "socks5h://user:pass@bastion.example.com:1080"
```

Every setting can be overridden by an env var, which wins over the file (and
can't be edited in the TUI while set):

| Setting  | Env var                   |
| -------- | ------------------------- |
| URL      | `LEEROY_JENKINS_URL`      |
| Username | `LEEROY_JENKINS_USERNAME` |
| Token    | `LEEROY_JENKINS_TOKEN`    |
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

Passwords in proxy URLs are masked on screen and in logs.

### Connecting

Leeroy connects on startup and reconnects whenever a setting that affects the
connection changes (URL, credentials, TLS, proxy). The header shows the state:
connecting, connected (Jenkins version and user), or the reason it failed.
Credentials are only sent when both username and API token are set; otherwise
Leeroy connects anonymously.

### TLS

Certificates are checked against the operating system's trust store, so a
company CA installed system-wide just works. For self-signed or otherwise
invalid certificates, `skip_tls_verify = true` (toggle "Skip TLS verify" in the
settings) disables verification. **This is insecure**: anyone between you and
Jenkins can read your API token. It's off by default, and while it's on the
header shows `⚠ TLS NOT VERIFIED`.
