---
name: verify-tui
description: Launch, drive and screenshot the Leeroy TUI to verify a change works — snapshot tests, the real binary in tmux (send keys, capture screen), and VHS PNG screenshots. Use when asked to run, test, verify or screenshot the app.
argument-hint: "[tape-name]"
allowed-tools: Bash(scripts/tui.sh *), Bash(scripts/screenshot.sh *), Bash(nix develop -c cargo *), Bash(cargo *)
---

Verify the current state of Leeroy. Stop and report at the first failure, quoting output.

1. Headless tests:
   `nix develop -c cargo test` — on snapshot changes, read the `.snap.new` files; accept with
   `nix develop -c cargo insta accept` only if the change is intended.
2. Real binary:
   ```sh
   scripts/tui.sh start
   scripts/tui.sh wait-for "Leeroy"
   scripts/tui.sh capture
   ```
   Runs are isolated (own config, no inherited `LEEROY_*`/`*_proxy` vars); pass env with
   `scripts/tui.sh start -e VAR=VAL`, a config with `-- --config FILE`.
   For connection features, run `scripts/fake-jenkins.py` in the background
   (`--auth`, `--delay`, `--status`, `--tls` simulate cases) and point the URL at it.
   Exercise the feature with `scripts/tui.sh keys <tmux-key-names>` (`Escape`, not `Esc`)
   and `capture` after each step. Check `scripts/tui.sh status` and `target/tui.log` for
   crashes. Always finish with `scripts/tui.sh stop`.
3. Visuals: `scripts/screenshot.sh $ARGUMENTS` then Read each printed PNG path and check
   layout, colors and alignment. For a new screen, first add `tapes/<name>.tape`
   (copy `tapes/help.tape`).

Report what was checked and what was seen, per layer.
