# Leeroy – a TUI for Jenkins

> *All right, chums! Let's do this! LEEROOOOOOOOOOOOOY! JEEEENKIIINS!*
>
> — Leeroy Jenkins

The jobs of an instance, with their last result and running builds:

![Jobs tab: every job with its last result](docs/screenshots/jobs.png)

The build history of all jobs, newest first:

![Builds tab: recent builds across all jobs](docs/screenshots/builds.png)

A build's details, here a running one with its progress:

![Build details of a running build](docs/screenshots/build.png)

Pipelines (jobs that trigger other jobs) with the status of their latest run:

![Pipelines tab: each pipeline with its latest run's status](docs/screenshots/pipelines.png)

A run: the builds it triggered, as a tree or (with `v`, shown here) as boxes:

![Run view: a pipeline's latest run as boxes](docs/screenshots/run.png)

## Installing

Download the archive for your platform from the
[releases](https://github.com/hbro/leeroy/releases) (`SHA256SUMS` lists the
checksums) and put the `leeroy` binary on your `PATH`:

| Platform | Archive |
| --- | --- |
| Linux x86_64 | `leeroy-vX.Y.Z-x86_64-linux-musl.tar.gz` or `…-x86_64-linux-glibc.tar.gz` |
| Linux ARM64 | `leeroy-vX.Y.Z-aarch64-linux-musl.tar.gz` or `…-aarch64-linux-glibc.tar.gz` |
| macOS Apple Silicon | `leeroy-vX.Y.Z-aarch64-darwin.tar.gz` |
| macOS Intel | `leeroy-vX.Y.Z-x86_64-darwin.tar.gz` |
| Windows x86_64 | `leeroy-vX.Y.Z-x86_64-windows-msvc.zip` |

On Linux, the `musl` builds are static and run on any distribution (Alpine
included); the `glibc` builds link against glibc 2.35 or newer. macOS binaries
aren't signed: clear the quarantine flag with
`xattr -d com.apple.quarantine leeroy`.

Or build from source: `cargo install --git https://github.com/hbro/leeroy`.
`leeroy --version` prints the version.

## Configuration

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

[jenkins.headers]  # see "Authentication" below; the file is written with mode 0600
Authorization = "Basic bWU6czNjcmV0"

[proxy]
url = "socks5h://user:pass@bastion.example.com:1080"

[refresh]
auto = true    # default: start with auto-refresh on (toggle with R at any time)
interval = 10  # default; seconds between automatic refreshes, minimum 1

[ui]
confirm_quit = true  # default: ask before quitting with q
theme = "auto"       # default; or "dark" / "light", see "Colours" below
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

### TLS

Certificates are checked against the operating system's trust store, so a
company CA installed system-wide just works. For self-signed or otherwise
invalid certificates, `skip_tls_verify = true` (toggle "Skip TLS verify" in the
settings) disables verification. **This is insecure**: anyone between you and
Jenkins can read your credentials. It's off by default, and while it's on the
header shows `⚠ TLS NOT VERIFIED`.

### Colours

Leeroy keeps your terminal's own background and colour scheme; the theme only
picks which of its colours go where. `dark` suits dark terminal backgrounds,
`light` light ones (inverted bars, a light selection, darker yellow and green).
The default, `auto`, asks the terminal for its background colour at startup and
picks accordingly; terminals that don't answer (tmux, for one) get `dark`. The
settings view shows what was detected. Change it with Enter on "Theme" in the
settings (it applies right away), `ui.theme` in the file, or `LEEROY_UI_THEME`.

## Using Leeroy

Content lives in tabs, selected with the digit keys shown in the tab bar:

| Key | Tab |
| --- | --- |
| `1` | Jobs |
| `2` | Builds (build history) |
| `3` | Pipelines |
| `4` | Pipeline runs (of all pipelines) |
| `0` | Settings (far right of the tab bar) |

Global keys, available everywhere except while typing in a field (the full list
is in the help popup, `h` or `?`, hinted at the right of the header):

| Key | Action |
| --- | --- |
| `q` | quit (asks first: `y`/`Enter`/`q` quits, `n`/`Esc` stays; setting `ui.confirm_quit`) |
| `Ctrl-C` | quit immediately, from anywhere |
| `h` / `?` | help |
| `o` | open what's on screen (job, build, console, pipeline, run) in Jenkins' web UI |
| `i` | about the Jenkins instance: version, user, nodes, executors, queue, and how Leeroy connects (TLS, proxy, header names) |
| `r` | refresh now (reconnects if the connection failed) |
| `R` | toggle auto-refresh for this session |

### Jobs

All jobs of the instance, with folders and multibranch projects flattened into
`folder/sub/job` names, their last result (colour *and* word) and `⟳` while a
build runs.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | navigation |
| `Enter` | details of the job's most recent build |
| `b` | start a build of the job (asks first; default parameter values) |
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
progress bar. For a Pipeline (Jenkinsfile) build, its stages are shown in order,
`● Checkout 4s ─▶ ● Build 1m 02s ─▶ ✕ Test 38s ─▶ ○ Deploy`, when Jenkins has the
Pipeline Stage View plugin (its `wfapi`); without it the section is left out. `r` and auto-refresh re-fetch the build while this view is open (so
a running build updates live), and the header's `⟳` age refers to it.

| Key | Action |
| --- | --- |
| `←` / `→` | older / newer build (deleted builds are skipped) |
| `Home` / `End` | first (oldest kept) / latest build |
| `c` | console output of this build |
| `b` | start a new build of this job (asks first) |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | navigation |
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
| `↑/↓` `j/k`, `PgUp/PgDn` | navigation |
| `Home` / `End` | top / bottom (`End` = follow again) |
| `←/→` | scroll long lines sideways |
| `c` / `Esc` | back to the build (`c` toggles the console) |
| `b` | start a new build of this job (asks first) |

Colour codes are removed, `\r` progress bars show their final state, and at
most the last 100 000 lines are kept.

### Builds

The build history of all jobs, newest first: status, job and build number, when
it started and how long it took (`…` while running). `Enter` opens the build (with
`←/→`, `c` etc. as usual); `Esc` comes back here. `b` starts a new build of the
selected build's job (asks first). `/` filters like on the Jobs tab.

Jenkins has no "newest builds overall" API, so Leeroy asks every job for its
newest *N* builds in **one** request (`tree=jobs[…,builds[…]{0,N}]`), where *N* is
the number of rows that fit on your screen, and merges them. That's exact for the
*N* rows shown. Moving down past the last row loads the next screenful (again one
request). Filtering by job name or `#number` is complete without extra requests;
filtering by result (e.g. "failed") only searches what's loaded.

### Pipelines

A pipeline is a job whose own builds (not triggered by another job) trigger
other jobs, e.g. an SCM-triggered `app/build` that starts `app/deploy`, which
starts `app/e2e`. A job the configuration says another job triggers (a
downstream or manual step) stays a step of that pipeline, even when it's started
by hand. The tab lists every pipeline with the status of its latest
run, when that started and how long it took (`…` while running). It's named
after the common prefix of its jobs (`app` for `app/build`, `app/deploy`, …, cut
at `/`, `-`, `_` or `.`), or after its first job when there's none or two
pipelines would share it.

A run's status: **failure**, **unstable** or **aborted** if any of its builds
ended that way (the worst wins); otherwise **success** when every part of the
pipeline (every job it can trigger) has a successful build in the run, and
**partial** while some part hasn't (still running, or not reached). `⟳` marks a
run that's still going.

| Key | Action |
| --- | --- |
| `↑/↓` `j/k`, `PgUp/PgDn`, `g/G` | navigation |
| `Enter` | open the latest run |
| `b` | start a new run (asks first; default parameter values) |
| `/` | filter by name |

Leeroy finds the relations in two places, because Jenkins records them in two
incomplete ways: the job configuration (freestyle "build other projects" and
`upstream` triggers) and the *upstream cause* of every triggered build (which
also covers a Jenkinsfile's `build job:` step). One request asks every job for
both, looking at its last 20 builds, so runs older than that aren't shown.

### Pipeline runs

Every run of every pipeline, newest first, named like its pipeline plus the
number of the build that started it (`app #12`). `Enter` opens it in the run
view; `/` filters by pipeline name or any `job #number` in the run.

### Run view

One run as a tree: every build under the build that triggered it, with when it
started and how long it took. `v` switches to boxes, stacked the same way (each
indented under its trigger, with arrows into it, borders in the build's status
colour) and back. When the boxes don't fit, the view scrolls to keep the
selected one in sight and the title says where there's more (`more ↓`). Parts
of the pipeline the run never reached are shown dimmed where they would have
run, under the job that triggers them (`○ deploy/production  not reached`, or
`disabled`): that's why a run whose builds all succeeded can still be partial.

| Key | Action |
| --- | --- |
| `←` / `→` | older / newer run of the pipeline |
| `Home` / `End` | first (oldest loaded) / latest run |
| `v` | tree ⇄ boxes (view) |
| `↑/↓` `j/k` | navigation |
| `Enter` | open the selected build (details, console with `c`) |
| `p` | promote: the run's manual steps, `Space` selects several, `Enter` takes them |
| `b` | start a new run of the pipeline (asks first) |
| `Esc` | back to the list |

### Promotions

`p` in the run view lists the run's manual steps: every job downstream (in the
job configuration) of a successfully finished build of the run that it hasn't
triggered, e.g. `shop/test #49 → shop/deploy` for a Build Pipeline plugin
"Manually Execute Downstream Project" step. `Space` selects one or more, `Enter`
takes the selected ones; nothing happens until at least one is selected. Only steps
that can be taken now are offered (after dev → acc → prod's dev, only acc), and a
step just taken isn't offered again until its build shows up.

Leeroy takes a step the way the Build Pipeline plugin's own page does, through a
Build Pipeline view (at the top level or in a folder): the new build then has
the upstream build as its cause
(it joins the run) and gets its parameters. Without such a view, the job is
started directly with the upstream build's parameters; it runs, but isn't linked
to the run (the notice says so).

## Development

`CLAUDE.md` describes the architecture and how to verify changes (tests,
snapshots, tmux, screenshots). CI (`.github/workflows/ci.yml`) runs rustfmt and
clippy, and the tests on Linux (x86_64 glibc and musl, ARM64), macOS and
Windows, for every push to `main` and every pull request.

### Releasing

Leeroy follows [semantic versioning](https://semver.org); the version in
`Cargo.toml` is the only source of truth. Record changes under
`## [Unreleased]` in `CHANGELOG.md` as you go. To release:

```sh
scripts/release.sh 0.2.0    # sets the version, dates the changelog, runs the checks
git commit -am "Release v0.2.0"
git tag -a v0.2.0 -m "Leeroy v0.2.0"
git push origin main v0.2.0
```

The tag triggers `.github/workflows/release.yml`: it fails if the tag doesn't
match `Cargo.toml`, runs the full CI (nothing is built or published unless
it passes), builds the seven targets, and publishes a GitHub release with
the archives, `SHA256SUMS` and the changelog section as release notes (a
version with a `-suffix` becomes a pre-release). Running the workflow by hand
(Actions → Release → Run workflow) is a dry run: it builds and uploads the
archives as workflow artifacts but publishes nothing.
