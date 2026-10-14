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

The level comes from `RUST_LOG` (default `info`: connections, fetches, builds
started, errors).

```sh
RUST_LOG=leeroy=debug leeroy   # Leeroy's own debug output
RUST_LOG=debug leeroy          # also the HTTP and TLS libraries (very verbose)
```

At `debug`, every request logs its path, the response status, the final URL
after redirects, the `X-Jenkins` version and any `WWW-Authenticate` challenge,
and which header *names* were sent. Header values, passwords in URLs and what
you type in the settings are never logged; job names and host names are, so
look through a log before sharing it.

Also check the header (connection state, `⚠ TLS NOT VERIFIED`), the settings
view (which proxy and theme are in effect) and `i` (how Leeroy connects).

## Common problems

### `authentication failed (HTTP 401/403 …)`

The message says whether Jenkins refused or a server in front of it, and which
login it asks for. Usual causes: a newline in the credentials (`echo` instead
of `printf '%s'`, a multi-line `pass` entry), or a redirect to another
scheme/host/port, which drops the `Authorization` header; the message then
names the address to use as the Jenkins URL. See
[Authentication](Connecting.md#authentication).

A 403 only when starting or re-running builds (`b`/`B`) means the user may
read but not build: it needs Jenkins' Job/Build permission.

### `untrusted TLS certificate (…)`

The certificate isn't signed by a CA in the operating system's trust store.
Install the company CA system-wide (best), or turn on "Skip TLS verify"
(insecure). See [TLS](Configuration.md#tls).

### `cannot connect: …` or `timed out`

Leeroy gives up after 10 seconds connecting and 20 seconds per request. Check
the URL, then the proxy: the settings view shows which one is used (an env var
like `https_proxy` counts too, unless `NO_PROXY` matches). If the Jenkins host
name only resolves inside the proxied network, use `socks5h://`, not
`socks5://`. See [Proxy](Connecting.md#proxy).

### `jenkins.username / jenkins.token … were replaced by custom headers` at startup

Those settings (and their env vars) were removed: authentication is now a
header. Remove them and add an `Authorization: Basic` header instead; the
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

### Fixed in earlier releases

If you see one of these, upgrade (`leeroy --version` shows yours):

| Symptom | Fixed in |
| --- | --- |
| The console of a running build with a long log opens somewhere in the middle | 0.5.2 |
| Jobs and Builds tabs sluggish on big instances, most of all while filtering | 0.3.3 |
| Promotions on current Jenkins start the job directly (not part of the run); Build Pipeline views in folders not found | 0.3.2 |
| A step started by hand shows up as a pipeline of its own | 0.3.2 |
| Pipelines and Pipeline runs sluggish on big instances | 0.3.1 |
| A queued promotion is offered (and can be started) twice | 0.3.1 |

## Reporting a problem

Open an [issue](https://github.com/hbro/leeroy/issues) with `leeroy --version`,
your Jenkins version (`i`), what you did and what you saw, and the relevant
part of a `RUST_LOG=leeroy=debug` log.
