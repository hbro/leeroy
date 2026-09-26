# leeroy

Terminal UI for Jenkins, written in Rust (ratatui 0.30 + crossterm 0.29 + tokio).

## Environment

NixOS: the toolchain comes from `flake.nix` (`direnv allow` once, or prefix commands
with `nix develop -c`). The system `cargo` has no `rustc`. The scripts in `scripts/`
enter the devShell by themselves when needed.

## Architecture

Elm-style: pure core, thin IO shell.

- `src/app.rs` — `App` state + `Action` enum + `App::update(Action)`. **No IO.**
- `src/event.rs` — `map_key(&App, KeyEvent) -> Option<Action>`.
- `src/ui.rs` — `render(&mut Frame, &App)`. **Deterministic**: no clock, randomness,
  or env reads; anything time-based must come in through `App` fields.
- `src/main.rs` — terminal setup, `tokio::select!` loop over key events, a tick
  interval and an `mpsc<Action>` channel. Async work (Jenkins API calls) should run
  in spawned tasks that send `Action`s back through that channel.

Rules:
- New behaviour = new `Action` variant + `update` arm + tests.
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
   scripts/tui.sh start [100x30]
   scripts/tui.sh wait-for "Jenkins TUI"
   scripts/tui.sh keys '?'        # tmux key names: Escape Enter Up Down Tab C-c ...
   scripts/tui.sh capture         # --ansi to include color escape codes
   scripts/tui.sh status          # running | exited (<code>) | stopped
   scripts/tui.sh stop
   ```
   Gotcha: `Esc` is NOT a tmux key name (it types E, s, c) — use `Escape`.
   App log for the run: `target/tui.log` (RUST_LOG=debug).

3. **Pixel screenshots** (colors/layout) — VHS
   ```sh
   scripts/screenshot.sh          # all tapes/*.tape; or: scripts/screenshot.sh help
   ```
   Then view `target/screenshots/<name>.png` with the Read tool. Add a tape per new
   screen; start it with `Source tapes/_settings.tape`, keep the app launch `Hide`n,
   and add a `Sleep` after `Screenshot` (a tape with zero shown frames fails).
   VHS is pinned to 0.11.0 in `flake.nix`: 0.12.0 silently writes nothing
   (charmbracelet/vhs#787). The script checks that screenshots were produced.
