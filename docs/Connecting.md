Leeroy connects on startup and reconnects whenever a setting that affects the
connection changes (URL, authentication, headers, TLS, proxy) and is saved. The
header shows the state: logging in, connecting, connected (Jenkins version and
user) or not connected; the settings (`0`) show why it isn't, in full, under
Connection.

## Authentication

The easiest way: enter the Jenkins URL in the settings (`0`) and press Enter on
**Detect login**. Leeroy asks the URL without credentials, follows where it's sent
and tells you what it found: a Jenkins anyone can read, a Jenkins login (its own
users, or an SSO plugin: then you use an API token, and `o` opens the page to
create one), basic auth, or a single sign-on proxy that logs in through
Keycloak. For a proxy it also checks whether its client can be used for a
device login, and if not, says what to ask your Keycloak admin. Enter then uses
what it found and logs in (asking for a user name and password first where
needed).

Or pick it yourself with **Authentication**, or `auth.mode` in the config file:

| Mode | For | Leeroy sends |
| --- | --- | --- |
| `headers` (default) | anything else: you set the headers yourself | only the [custom headers](#custom-headers) |
| `anonymous` | a Jenkins that lets anyone read | nothing |
| `basic` | a Jenkins user with a password or **API token**, or a proxy with HTTP basic auth | `Authorization: Basic …` |
| `oidc` | a proxy in front of Jenkins that logs in through **Keycloak** (or another OpenID Connect provider) and accepts bearer tokens | `Authorization: Bearer <access token>` |

The custom headers are sent in every mode, except an `Authorization` header:
with `anonymous`, `basic` or `oidc` it isn't sent (the settings say so), so a
header you used before can stay until the new mode works.

**Which one for Keycloak?** It depends on where the login happens:

- **Jenkins itself logs in through Keycloak** (its login page redirects to
  Keycloak; the OpenID Connect or Keycloak plugin): use `basic` with a Jenkins
  **API token**. Log in to Jenkins in the browser once, then create a token
  under your user → Security → API Token. Jenkins doesn't accept Keycloak's
  tokens on its API.
- **A proxy in front of Jenkins logs in through Keycloak** (e.g. oauth2-proxy):
  use `oidc`. The proxy has to accept bearer tokens (oauth2-proxy:
  `--skip-jwt-bearer-tokens`); otherwise Leeroy reports that it was
  redirected to a login page.

### Basic

```toml
[auth]
mode = "basic"
username = "me"
password_store = "keyring"   # keyring (default) | command | file
```

Where the password (or API token) comes from:

| `password_store` | Password |
| --- | --- |
| `keyring` (default) | the OS keyring: Secret Service (GNOME Keyring, KWallet) on Linux, the Keychain on macOS, the Credential Manager on Windows. Set it with the **Password** row in the settings. |
| `command` | the first line printed by `password_command`, run at every connect, e.g. `pass show work/jenkins` |
| `file` | `password` in the config file (plain text; the file is mode 0600). Set it with the **Password** row. |

`LEEROY_AUTH_PASSWORD` wins over all of them (and is never saved). The
password is shown while you type it in the settings, and masked otherwise;
leaving it empty removes it.

The password command gets no terminal: a GPG pinentry has to be graphical, or
the key cached by gpg-agent (see
[Troubleshooting](Troubleshooting.md#password-command-fails)).

### Keycloak / OpenID Connect

```toml
[auth]
mode = "oidc"
issuer = "https://sso.example.com/realms/acme"   # Keycloak: the realm URL
client_id = "leeroy"
login_flow = "device"   # device (default) | browser
token_store = "keyring" # keyring (default) | file | memory
```

When there's no (valid) login, Leeroy shows a login window:

- **device** (default): open the address it shows in any browser, on any
  machine, and enter the code (`o` opens it here). Works over SSH.
- **browser**: Leeroy opens the browser and receives the answer on
  `127.0.0.1`, so the browser has to run on the same machine.

`b` switches between the two, `Esc` cancels (`r` tries again). Once logged in,
Leeroy refreshes the access token before it expires and keeps the refresh
token for the next start: in the OS keyring, in a private `tokens.toml` in
Leeroy's state directory (`token_store = "file"`, next to the log), or
nowhere (`memory`: log in at every start). **Detect login** in the settings
forgets it, e.g. to log in as someone else.

The Keycloak client needs:

- Client authentication **off** (a public client: Leeroy can't keep a client
  secret).
- **OAuth 2.0 Device Authorization Grant** on, for the device login.
- Standard flow on with valid redirect URI `http://127.0.0.1/*`, for the
  browser login (Leeroy picks a free port each time).

Other providers work the same way: Leeroy finds the endpoints through the
issuer's `/.well-known/openid-configuration`.

### Custom headers

Leeroy sends the HTTP headers under `[jenkins.headers]` with every request, in
every mode (an `Authorization` header only in mode `headers`), e.g. `X-Forwarded-User: <user>` for a trusted reverse proxy or an API
key. Add them in the settings view (`+ add header`, edit as `Name: value`,
clear the field to remove one). Header values are masked on screen (except in
the field you're editing) and never logged.

With mode `headers`, an `Authorization` header logs in too:
`Authorization: Basic <base64 of user:password>` (make it with
`printf '%s' 'user:password' | base64 -w0`) or `Authorization: Bearer <token>`.
The `basic` mode does the encoding for you.

Env-provided headers (`LEEROY_JENKINS_HEADERS_<NAME>`, `-` written as `_`) win
over the file and are read-only in the TUI. Credentials embedded in the
Jenkins URL (`https://user:pass@host`) also work, but end up in the config
file; they are masked wherever the URL is shown, except while you edit it.

Refused with a 401/403: see
[Authentication fails](Troubleshooting.md#authentication-fails).

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

Without a proxy setting, the usual env vars apply, like with curl:
`https_proxy`/`HTTPS_PROXY` (or `http_proxy`/`HTTP_PROXY` for an `http://`
Jenkins URL), then `all_proxy`/`ALL_PROXY`, except for hosts listed in
`no_proxy`/`NO_PROXY` (host names or domain suffixes, or `*`). The settings
view shows which one is in effect. An explicit Leeroy proxy setting ignores
`NO_PROXY`.

Passwords in proxy URLs are masked on screen (except while you edit the field)
and in logs. Can't connect through the proxy: see
[Cannot connect](Troubleshooting.md#cannot-connect).
