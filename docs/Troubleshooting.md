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

Also check the header (connection state, `⚠ TLS NOT VERIFIED`), the settings
view (which proxy and theme are in effect) and `i` (how Leeroy connects).

## Common problems

### Authentication fails

The error, `authentication failed (HTTP 401/403 …)`, says whether Jenkins or a
server in front of it refused, and which login it asks for. Usual causes:

- The credentials contain a newline (`echo … | base64` instead of
  `printf '%s' …`, or a multi-line `pass` entry). Leeroy rejects such `Basic`
  values up front.
- A redirect to another scheme, host or port (e.g. `http://` → `https://`):
  the `Authorization` header is dropped on such redirects for safety. The
  error names the redirect target; use that as the Jenkins URL.
- A 403 only when starting or re-running builds (`b`/`B`): the user may read
  but not build; it needs Jenkins' Job/Build permission.

With `RUST_LOG=debug` (see [More detail](#more-detail)) the log shows which
header *names* were sent, the final URL and the response status. See
[Authentication](Connecting.md#authentication).

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
custom headers`. Those settings (and their env vars) were removed in favour of
headers. Remove them and add an `Authorization: Basic` header instead; the
message shows how. See [Authentication](Connecting.md#authentication).

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

Pipelines and their runs are worked out from the last 20 builds of every job,
so older runs aren't listed.

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
