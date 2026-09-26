# Leeroy

Terminal UI for Jenkins, written in Rust (ratatui 0.30 + crossterm 0.29 + tokio).

## Environment

NixOS: the toolchain comes from `flake.nix` (`direnv allow` once, or prefix commands
with `nix develop -c`). The system `cargo` has no `rustc`. The scripts in `scripts/`
enter the devShell by themselves when needed.

## Architecture

Elm-style: pure core, thin IO shell.

- `src/app.rs` — `App` state + `Action` enum + `App::update(Action) -> Vec<Effect>`.
  **No IO**: side effects (saving config, connecting) are returned as `Effect`s, which
  `main.rs`'s `Executor` runs and answers with an `Action` (`SettingsSaved`,
  `ConnectFinished`). `App::start()` returns the startup effects.
- `src/jenkins.rs` — reqwest client; `get()` (headers, auth diagnostics, logging)
  behind `check()` (`whoAmI/api/json`) and `fetch_jobs()` (`api/json?tree=…`). Proxy
  is always passed in explicitly (`.no_proxy()`), so behaviour matches the UI.
- `src/jobs.rs` — job model, `/api/json` parsing (folders flattened, not listed),
  filter matching, `JobsState` (load state, filter, selection). Pure.
- `src/builds.rs` — last-build model/parsing, `BuildView` state. Request paths
  are built from the job's full name (`job/a/job/b/lastBuild/…`) against the
  configured URL, never from Jenkins' `url` field (may be an internal address
  behind a reverse proxy). 404 on lastBuild = never built (`Ok(None)`).
- `src/history.rs` — Builds tab: one request asks every job for its newest N
  builds (`builds[…]{0,N}` in the jobs tree), merged by start time; only the first
  N are shown (exact). N = visible rows (`App::list_rows`, recorded by the
  renderer); moving past the end fetches N + one page. Job-name/`#number` filters
  are complete client-side.
- `src/console.rs` — console output: `ConsoleView` buffer fed by
  `logText/progressiveText?start=<byte>` chunks (`X-Text-Size` = next offset,
  `X-More-Data` = still running), UTF-8 carried across chunk boundaries, lines
  cleaned (ANSI/control chars stripped, text after the last `\r`, tabs), capped at
  `MAX_LINES`. Follow = pinned to the bottom; scrolling up pauses, `End` resumes.
- `src/config.rs` — config file location, load/save, env var overrides. Takes the
  environment as a parameter (never reads `std::env` itself) so it's testable.
- `src/event.rs` — `map_key(&App, KeyEvent) -> Option<Action>`.
- `src/ui.rs` — `render(&mut Frame, &App)`. **Deterministic**: no clock, randomness,
  or env reads. Time enters only via `Action::Tick(Instant, SystemTime)` →
  `App::now` (monotonic: refresh timing, data age) and `App::wall_now` (build
  start times). Tests set both. The renderer may *record* layout facts through
  `Cell`s (e.g. `BuildView::max_scroll`) for `update` to clamp against.
- `src/main.rs` — terminal setup, `tokio::select!` loop over key events, a tick
  interval and an `mpsc<Action>` channel. Async work (Jenkins API calls) should run
  in spawned tasks that send `Action`s back through that channel.

Rules:
- New behaviour = new `Action` variant + `update` arm + tests.
- Header: app name + connected Jenkins instance (`App::connection`).
- Tabs: `app::Tab`, selected with digits: content tabs `1`–`9` (`Tab::key`),
  Settings `0`, right-aligned in the tab bar. New content = new `Tab` + `View`
  variant. Tab keys work everywhere except text input (digits are typed there).
  Tabs aren't closable: `Esc` doesn't leave them.
- Colours: never hard-code a `Color` in `ui.rs`; use a role of `theme::Theme`
  (`let t = app.theme();`, re-resolved every frame, so switching is live). A new
  role goes in both `DARK` (the original look, keep it) and `LIGHT`. Themes never
  paint the background: they use the terminal's own, plus its 16 colours
  (`LIGHT` adds two from the 256-colour cube where ANSI yellow/green are
  unreadable on white). `ui.theme` = auto|dark|light (`SettingKey::choices`:
  Enter cycles, first = default = unset); auto resolves against
  `App::terminal_appearance`, detected in `main.rs` before raw mode
  (terminal-colorsaurus; `None` = dark). Selected rows use `t.selected_bg`;
  text colours go through `t.on_selected()` there. Tests in `theme.rs` check
  every theme for fg != bg; `selected_row_status_stays_readable` renders both.
- Global keys (`GLOBAL_BINDINGS`; no bottom bar, listed in the help popup, the
  header shows an `h/?` hint): q, h/?, r (refresh), R (toggle auto-refresh). The refresh status is a compact `⟳ 4s` at the right end of the
  header (icon green = auto-refresh on, gray = off; `…` fetching, `✕` failed).
- Build view (`View::Build`, part of the Jobs tab): `Enter` on a job opens it
  and fetches unconditionally. `BuildRef::Latest` fetches the job's build numbers
  (`allBuilds`, gaps!) + lastBuild in one request; `←/→ Home/End` step through
  those numbers (`BuildView::step`; the newest is always `Latest` so it follows
  new builds), each step fetching `BuildRef::Number(n)` immediately. Answers are
  also matched on `which`, so quick steps ignore builds left behind; `r`/auto-refresh re-fetch what's on screen (build
  view: the build, not the job list) under the same one-in-flight rule. Results
  are tagged with connection generation + job name; mismatches are ignored.
- Build views remember where they came from (`BuildView::origin`: Jobs or
  Builds): Esc returns there and `App::tab()` highlights that tab. Opened at a
  number (from Builds), the first fetch also gets the job's build numbers
  (`want_numbers`) so ←/→ work.
- Console view (`View::Console`, under the build view): `c` toggles it (opens it
  from the build view, `c`/`Esc` go back) for the
  shown build's *number*. Polled every `CONSOLE_POLL` (1s) while shown and the
  build runs, regardless of auto-refresh; one fetch in flight; chunks are matched
  on job + number + start offset (duplicates never appended). Only visible lines
  are rendered; the renderer records the viewport height for paging.
- Jobs: fetched after each successful connect and on `r`, tagged with the
  connection generation (stale results ignored; list cleared on reconnect). A
  reload keeps the old list visible (`refreshing`) and restores the selection by
  name. Auto-refresh (`R`, runtime `App::auto_refresh`, default from
  `refresh.auto`) fetches on `Tick` when connected, idle and one
  `refresh.interval` after the last *attempt* (so failures back off).
  Never more than one fetch in flight: `fetch_jobs()` is a no-op while
  `JobsState::fetch_in_flight()` (manual `r` included); only a new connection
  (new generation) resets that.
- Bottom of screen = the context bar: keys of
  whatever has focus, from `context_bindings(App::context())`. A new view or overlay
  = new `View`/`Context` variant, its bindings in `event.rs`, handled in `map_key`.
  Test `advertised_bindings_are_mapped` fails if a shown key does nothing.
- `Esc` = `Action::Back`: closes the current context (popup, sub-view) and is
  only mapped where `Context::closable()`. It must never quit the app.
- Text input contexts (`Context::captures_input()`) receive every key except
  `Ctrl-C`; global keys are off there (generally: `Context::global_keys_off()`,
  which also covers the quit prompt).
- Quit: `q` = `Action::RequestQuit` → "Quit Leeroy?" prompt (`Context::ConfirmQuit`,
  on top of everything; y/Enter/q quit, n/Esc stay) unless `ui.confirm_quit` (default
  on) is off. `Ctrl-C` = `Action::Quit`, immediate, everywhere. Editing uses
  `input::TextInput` (char-indexed cursor): ←/→, Home/End, Ctrl-A/E, Backspace,
  Delete, Ctrl-U. ↑/↓ while editing = save the field (if valid) and move. The
  field being edited is shown exactly as typed (no masking, per user request);
  masking applies only to fields that aren't being edited.
- Settings: add a `SettingKey` variant (label, TOML path, env var
  `LEEROY_<TABLE>_<KEY>`, `section`, `doc`, `validate`, `display`) plus a field in
  `Settings`. The view groups rows by `config::Section`: Jenkins (URL, TLS, proxy,
  then the Headers sub-section) and Application (refresh, confirm quit). Tests and
  tapes navigate settings by row, so select rows explicitly rather than wrapping. `doc` is shown in the settings view and written as a TOML comment
  when the key is first saved. Invalid values keep the edit open; invalid file/env
  values fail at startup. Precedence: env var >
  config file; env-set values are read-only in the TUI and never written to the file.
  Config path: `--config` > `$LEEROY_CONFIG` > `$XDG_CONFIG_HOME/leeroy/config.toml`
  (default `~/.config/...`); legacy `~/.leeroy/config.toml` only if it exists and
  the XDG file doesn't. Windows: `%APPDATA%\leeroy\config.toml` instead of the
  `~` paths (log: `%LOCALAPPDATA%`). Path logic takes a `config::Platform`
  parameter so every platform's rules are tested on Linux; never use `cfg!` for
  it outside `Platform::current()`. Empty env values count as unset.
- Auth is generic: `[jenkins.headers]` (`Settings::header`/`set_header`, names
  case-insensitive) sent with every request; env `LEEROY_JENKINS_HEADERS_<NAME>`
  (`_` → `-`). Settings view rows = `SettingsState::rows()`: fixed `SettingKey`s,
  then one row per header, then `+ add header` (edit `Name: value`, empty =
  delete). Removed `jenkins.username`/`token` must keep failing loudly at startup.
  `Authorization: Basic` values are decoded and rejected when not base64, without
  `:`, or containing a newline (echo / multi-line `pass show`). 401/403 errors say
  who refused (`X-Jenkins` present = Jenkins) and whether a cross-origin redirect
  dropped the header (reqwest strips `Authorization` then); keep that.
- Secrets: header values are masked in the UI (fixed `••••••••`) except in the
  field currently being edited, sent as sensitive `HeaderValue`s, and only header *names* appear
  in `Debug`. `Input` actions are never logged. Passwords in the Jenkins and proxy
  URLs go through `config::redact_url` everywhere they're shown (the header's
  `ConnectionStatus` stores the redacted URL). Keep it that way.
- Connection: every attempt bumps `App::connection_generation`; `ConnectFinished`
  for an older generation is ignored and the executor aborts the previous task.
  Reconnect happens only when `SettingsState::connection_config()` changes.
- Bool settings (`SettingKey::is_bool`): Enter toggles; the default value
  (`SettingKey::default_on`) = key removed, the other value is stored
  as TOML bool, env accepts true/false/1/0/yes/no/on/off. `skip_tls_verify` must
  stay off by default and show the header warning when on.
- Proxy: `proxy.url`, scheme decides type/DNS (`socks5h`/`socks4a` = remote DNS).
  Unset → `src/proxy.rs` picks the conventional `*_proxy` env var (curl rules, incl.
  `NO_PROXY`) from a `ProxyEnv` snapshot taken at startup.
- "Leeroy" is capitalized in all prose/UI text; only the crate/binary name,
  paths, env vars and tmux ids stay lowercase `leeroy`.
- Import crossterm types via `ratatui::crossterm` (EventStream is the exception:
  it needs the direct `crossterm` dep for the `event-stream` feature; both are the
  same crate version — keep them in sync when upgrading ratatui).
- Never print to stdout/stderr while the TUI runs; use `tracing` (logs go to
  `$LEEROY_LOG`, default `~/.local/state/leeroy/leeroy.log`; level via `RUST_LOG`).

## Versioning and CI

- SemVer; `Cargo.toml` `version` is the single source. User-visible changes get
  a line under `## [Unreleased]` in `CHANGELOG.md` (Keep a Changelog). Releases:
  `scripts/release.sh X.Y.Z`, then commit + tag `vX.Y.Z` + push; the tag must
  match `Cargo.toml` (release workflow checks).
- CI runs tests on Linux, macOS and Windows: keep tests platform-neutral (build
  expected paths with `Path::join`, `#[cfg(unix)]` only for truly Unix-only
  behaviour like file modes/symlinks). `.gitattributes` forces LF so snapshots
  match on Windows. Validate workflow edits with
  `nix shell nixpkgs#actionlint nixpkgs#shellcheck -c actionlint`.

## Verifying changes

Run all three layers for UI changes; layer 1 is mandatory for every change.

1. **Tests + snapshots** (fast, headless)
   ```sh
   cargo test                     # unit tests + tests/render.rs snapshots (80x24)
   cargo insta review             # or: cargo insta accept, after checking .snap.new
   cargo clippy --all-targets -- -D warnings && cargo fmt --check
   ```
   Add a snapshot case in `tests/render.rs` for every new screen/state. Snapshots
   capture text only, not colors — use layer 3 for styling.

2. **Real binary in tmux** (interactive, real terminal) — `scripts/tui.sh`
   ```sh
   scripts/tui.sh start [100x30] [-e VAR=VAL]... [-- app args]
   scripts/tui.sh wait-for "Leeroy"
   scripts/tui.sh keys '?'        # tmux key names: Escape Enter Up Down Tab C-c ...
   scripts/tui.sh capture         # --ansi to include color escape codes
   scripts/tui.sh status          # running | exited (<code>) | stopped
   scripts/tui.sh stop
   ```
   Gotcha: `Esc` is NOT a tmux key name (it types E, s, c) — use `Escape`.
   App log for the run: `target/tui.log` (RUST_LOG=debug).
   Fake Jenkins for connection tests: `scripts/fake-jenkins.py [--port 8099]
   [--auth USER:TOKEN] [--delay SECS] [--status CODE] [--tls] [--jobs N] [--churn]`
   (job tree with folders/multibranch/all statuses, each job's builds when the tree
   asks for `builds[…]{0,N}`; `lastBuild` per job: 404 for
   never built, running builds progress in real time and their console log grows
   a line every 0.3s (ANSI, `\r`, UTF-8, long lines); `--churn` changes a status
   per request to watch auto-refresh).
   Run it in the background with its own process group; don't `pkill -f` it (the
   pattern can match unrelated shells).
   Isolation: all `LEEROY_*` and `*_proxy` vars from your shell are cleared and the config is a
   fresh `target/tui/config.toml` per start. Test env overrides with `-e`, a
   prepared config with `-- --config FILE`.

3. **Pixel screenshots** (colors/layout) — VHS
   ```sh
   scripts/screenshot.sh          # all tapes/*.tape; or: scripts/screenshot.sh help
   ```
   Then view `target/screenshots/<name>.png` with the Read tool. Add a tape per new
   screen; start it with `Source tapes/_settings.tape` (dark scheme; for another
   scheme `Set Theme` first, then `Source tapes/_common.tape`, as `light.tape`
   does: a second `Set Theme` is ignored), keep the app launch `Hide`n,
   and add a `Sleep` after `Screenshot` (a tape with zero shown frames fails).
   VHS 0.11 has no `Home`/`End` commands (they get typed as text): use `Left N`.
   VHS is pinned to 0.11.0 in `flake.nix`: 0.12.0 silently writes nothing
   (charmbracelet/vhs#787). The script checks that screenshots were produced.
   `tapes/readme.tape` writes the README screenshots to `docs/screenshots/`
   (committed, 960x540, mock instance `jenkins.example.com` via the fake as a
   proxy): regenerate them with `scripts/screenshot.sh readme` when the UI changes.
   Tapes run with a fresh `target/vhs/config.toml` and no `LEEROY_*`/`*_proxy` vars,
   against a fake Jenkins on http://127.0.0.1:8099 started by the script (never
   the real network).
