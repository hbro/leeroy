# Leeroy

Terminal UI for Jenkins, written in Rust (ratatui 0.30 + crossterm 0.29 + tokio).

## Environment

NixOS: the toolchain comes from `flake.nix` (`direnv allow` once, or prefix commands
with `nix develop -c`). The system `cargo` has no `rustc`. The scripts in `scripts/`
enter the devShell by themselves when needed.

## Architecture

Elm-style: pure core, thin IO shell.

- `src/app.rs` — `App` state + `Action` enum + `App::update(Action) -> Option<Effect>`.
  **No IO**: side effects (e.g. saving config) are returned as an `Effect`, which
  `main.rs` runs and answers with an `Action` (e.g. `SettingsSaved`).
- `src/config.rs` — config file location, load/save, env var overrides. Takes the
  environment as a parameter (never reads `std::env` itself) so it's testable.
- `src/event.rs` — `map_key(&App, KeyEvent) -> Option<Action>`.
- `src/ui.rs` — `render(&mut Frame, &App)`. **Deterministic**: no clock, randomness,
  or env reads; anything time-based must come in through `App` fields.
- `src/main.rs` — terminal setup, `tokio::select!` loop over key events, a tick
  interval and an `mpsc<Action>` channel. Async work (Jenkins API calls) should run
  in spawned tasks that send `Action`s back through that channel.

Rules:
- New behaviour = new `Action` variant + `update` arm + tests.
- Header: app name + connected Jenkins instance (`App::connection`).
- Bottom of screen = two bars. Global bar (last line): app-wide commands
  (`GLOBAL_BINDINGS`). Context bar above it: keys of
  whatever has focus, from `context_bindings(App::context())`. A new view or overlay
  = new `View`/`Context` variant, its bindings in `event.rs`, handled in `map_key`.
  Test `advertised_bindings_are_mapped` fails if a shown key does nothing.
- `Esc` = `Action::Back`: closes the current context (popup, sub-view) and is
  only mapped where `Context::closable()`. It must never quit the app.
- Text input contexts (`Context::captures_input()`) receive every key except
  `Ctrl-C`; global keys are off there and the global bar is dimmed.
- Settings: add a `SettingKey` variant (label, TOML path, env var
  `LEEROY_<TABLE>_<KEY>`, secret?) plus a field in `Settings`. Precedence: env var >
  config file; env-set values are read-only in the TUI and never written to the file.
  Config path: `--config` > `$LEEROY_CONFIG` > `$XDG_CONFIG_HOME/leeroy/config.toml`
  (default `~/.config/...`); legacy `~/.leeroy/config.toml` only if it exists and
  the XDG file doesn't. Empty env values count as unset.
- Secrets (API token) are masked in the UI, redacted in `Debug`, and `Input` actions
  are never logged. Keep it that way.
- "Leeroy" is capitalized in all prose/UI text; only the crate/binary name,
  paths, env vars and tmux ids stay lowercase `leeroy`.
- Import crossterm types via `ratatui::crossterm` (EventStream is the exception:
  it needs the direct `crossterm` dep for the `event-stream` feature; both are the
  same crate version — keep them in sync when upgrading ratatui).
- Never print to stdout/stderr while the TUI runs; use `tracing` (logs go to
  `$LEEROY_LOG`, default `~/.local/state/leeroy/leeroy.log`; level via `RUST_LOG`).

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
   Isolation: all `LEEROY_*` vars from your shell are cleared and the config is a
   fresh `target/tui/config.toml` per start. Test env overrides with `-e`, a
   prepared config with `-- --config FILE`.

3. **Pixel screenshots** (colors/layout) — VHS
   ```sh
   scripts/screenshot.sh          # all tapes/*.tape; or: scripts/screenshot.sh help
   ```
   Then view `target/screenshots/<name>.png` with the Read tool. Add a tape per new
   screen; start it with `Source tapes/_settings.tape`, keep the app launch `Hide`n,
   and add a `Sleep` after `Screenshot` (a tape with zero shown frames fails).
   VHS is pinned to 0.11.0 in `flake.nix`: 0.12.0 silently writes nothing
   (charmbracelet/vhs#787). The script checks that screenshots were produced.
   Tapes run with a fresh `target/vhs/config.toml` and no `LEEROY_*` vars.
