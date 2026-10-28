## Logs

Leeroy never prints while the TUI runs; it logs to a file instead:

| Platform | Log file |
| --- | --- |
| Linux, macOS | `$XDG_STATE_HOME/leeroy/leeroy.log` (`~/.local/state/leeroy/leeroy.log` when `$XDG_STATE_HOME` is unset) |
| Windows | `%LOCALAPPDATA%\leeroy\leeroy.log` |

`LEEROY_LOG=/path/to/file` overrides the location. New runs append to the
file, which is never rotated: delete it when it gets big.

Follow it from a second terminal while you reproduce a problem:

```sh
tail -f ~/.local/state/leeroy/leeroy.log
```

## More detail

How much is logged is set with `RUST_LOG`, the usual variable for Rust
programs (default `info`: connections, fetches, builds started, errors). Its
value is a level (`error`, `warn`, `info`, `debug`, `trace`), or a
comma-separated list of `source=level` directives for more control:

```sh
RUST_LOG=debug leeroy          # everything, including the HTTP and TLS libraries
RUST_LOG=leeroy=debug leeroy   # only Leeroy's own debug output, less noise
```

`RUST_LOG` applies to every Rust program, so one set in your shell for other
work affects Leeroy too.

At `debug`, every request logs its path, the response status, the final URL
after redirects, the `X-Jenkins` version and any `WWW-Authenticate` challenge,
and which header *names* were sent. Header values, passwords in URLs and what
you type in the settings are never logged; job names and host names are, so
look through a log before sharing it. Stay below `trace`, where libraries log
much more of what goes over the wire.

Also check the settings view (Connection: why it isn't connected; which proxy
and theme are in effect), the header (`⚠ TLS NOT VERIFIED`) and `i` (how
Leeroy connects).

## Common problems

### Authentication fails

The error, `authentication failed (HTTP 401/403 …)`, says whether Jenkins or a
server in front of it refused, and which login it asks for. Usual causes:

- Wrong user name or password. With a Jenkins that logs in through Keycloak
  (or another SSO), the password has to be a Jenkins API token.
- The credentials contain a newline (a multi-line `pass` entry, or
  `echo … | base64` for a hand-made header instead of `printf '%s' …`). Leeroy
  rejects those up front.
- A redirect to another scheme, host or port (e.g. `http://` → `https://`):
  the `Authorization` header is dropped on such redirects for safety. The
  error names the redirect target; use that as the Jenkins URL.
- A 403 only when starting or re-running builds (`b`/`B`): the user may read
  but not build; it needs Jenkins' Job/Build permission.

With `RUST_LOG=debug` (see [More detail](#more-detail)) the log shows which
header *names* were sent, the final URL and the response status. See
[Authentication](Connecting.md#authentication).

### Redirected to a login page

The error is `redirected to a login page (…)`: a server in front of Jenkins
(an SSO proxy) sent its browser login page instead of the API's answer. With
auth mode `oidc` it didn't accept the token: it has to allow bearer tokens
(oauth2-proxy: `--skip-jwt-bearer-tokens`, with Keycloak as a trusted issuer).
Otherwise, try auth mode `oidc`. See [Authentication](Connecting.md#authentication).

### No password in the OS keyring

`no password in the OS keyring yet`: set it with the Password row in the
settings (`0`). `no OS keyring available`: there's no keyring service (a
server without a desktop session, WSL, a container): use `password_store =
"command"` or `"file"` (and for OpenID Connect `token_store = "file"`), or
`$LEEROY_AUTH_PASSWORD`.

### Password command fails

`the password command failed (exit code N)`, `didn't finish`, or `printed
nothing`: run the command yourself to check it. Leeroy runs it without a
terminal, so it can't ask for anything there: with `pass`, the GPG key has to
be unlocked by gpg-agent already, or use a graphical pinentry
(`pinentry-program` in `~/.gnupg/gpg-agent.conf`). Its output is never shown
or logged; only the first line is used.

### Login fails or keeps asking

The login window stays open with the provider's reason and a hint; `r`
tries again, `b` switches between the code and the browser login.

- `device login refused`, `the identity provider doesn't offer device login`:
  turn on "OAuth 2.0 Device Authorization Grant" for the Keycloak client, or
  set `login_flow = "browser"`.
- `invalid_client`, `unauthorized_client`: the client ID is wrong, or the
  client isn't public (client authentication has to be off).
- The browser shows a Keycloak error about the redirect URI: add
  `http://127.0.0.1/*` to the client's valid redirect URIs.
- `no OpenID configuration at …`: the issuer URL is wrong; for Keycloak it's
  `https://<host>/realms/<realm>`.
- The login window comes back after a while: the Keycloak session ended
  (its SSO session idle or max time), so the refresh token stopped working.
  That's Keycloak's setting, not Leeroy's.
- With `token_store = "memory"`, Leeroy asks at every start by design.

### Untrusted TLS certificate

The error is `untrusted TLS certificate (…)`: the certificate isn't signed by
a CA in the operating system's trust store. Install the company CA
system-wide (best), or turn on "Skip TLS verify" (insecure). See
[TLS](Configuration.md#tls).

### Cannot connect

The error is `cannot connect: …` or `timed out` (Leeroy gives up after 10
seconds connecting and 20 seconds per request). Check the URL, then the proxy:
the settings view shows which one is used (an env var like `https_proxy`
counts too, unless `NO_PROXY` matches). If the Jenkins host name only resolves
inside the proxied network, use `socks5h://` (or `socks4a://`): with
`socks5://` the name is resolved on your machine first. See
[Proxy](Connecting.md#proxy).

### Username and token settings rejected at startup

The error starts with `jenkins.username / jenkins.token … were replaced by
the [auth] settings`. Those old settings (and their env vars) are gone:
remove them and set `auth.mode = "basic"` with `auth.username`; the message
shows how. See [Authentication](Connecting.md#authentication).

### Leeroy uses a different config file than expected

The settings view shows the path. `--config` and `$LEEROY_CONFIG` win over the
default location; when both `~/.config/leeroy/config.toml` and the legacy
`~/.leeroy/config.toml` exist, the first is used and a warning is printed (and
logged). See [Configuration](Configuration.md).

### Colours are hard to read

`auto` asks the terminal for its background colour; terminals that don't
answer (tmux, for one) get `dark`. Set the theme explicitly: Enter on "Theme"
in the settings, `ui.theme = "light"`, or `LEEROY_UI_THEME=light`. See
[Colours](Configuration.md#colours).

### A Pipeline build has no stages

Stages come from the Pipeline Stage View plugin. Without it (or when it fails)
the section is left out; the rest of the build view still works.

### Old pipeline runs are missing

The Pipelines and Pipeline runs tabs only look at the last 3 builds of every
job (**Pipeline builds** in the settings). Open the pipeline (Enter): that loads
its jobs' last 20 builds, and `←` goes back through them. Runs older than that
aren't shown. Raising **Pipeline builds** shows more runs in the Pipeline runs
tab, but can make these tabs much slower or time out on big instances (see
[Pipelines](Pipelines.md)).

### The Pipelines tab is slow or times out

Its request asks every job for its last **Pipeline builds** builds (default
3). On an instance with thousands of jobs Jenkins needs a while for that,
most of all the first time (it loads old builds from disk); the tab shows how
long it has been loading, and gives up after 2 minutes. Set **Pipeline
builds** back to 3, or lower (see [Pipelines](Pipelines.md) for what 1
misses).

### A promotion or re-run isn't part of the run

The notice says "not linked": Leeroy found no Build Pipeline view to go
through, so it started the job directly with the right parameters, but without
the upstream build as its cause. Create a Build Pipeline view (at the top level
or in a folder) that shows the pipeline. See [Promotions](Pipelines.md#promotions).

## Reporting a problem

Reproduce it with a fresh, full debug log:

```sh
RUST_LOG=debug LEEROY_LOG=leeroy-debug.log leeroy
```

Then open an [issue](https://github.com/hbro/leeroy/issues) with
`leeroy --version`, your Jenkins version (`i`), what you did and what you saw,
and `leeroy-debug.log` attached (look through it first, see above).
