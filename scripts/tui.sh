#!/usr/bin/env bash
# Drive the real Leeroy binary inside a detached tmux session.
# Uses a private tmux server socket, so it never touches your own tmux.
#
#   scripts/tui.sh start [WxH] [-- args...]  build + launch (default 100x30)
#   scripts/tui.sh keys <key>...              send tmux keys: q '?' Down Enter C-c
#   scripts/tui.sh capture [--ansi]           print screen (--ansi keeps colors)
#   scripts/tui.sh wait-for <text> [secs]     poll until text is on screen (default 5s)
#   scripts/tui.sh status                     running | exited (<code>) | stopped
#   scripts/tui.sh stop                       kill session
#
# Logs of the run go to target/tui.log.
set -euo pipefail

cd "$(dirname "$0")/.."
SOCKET=leeroy-agent
SESSION=leeroy
BIN=target/debug/leeroy
EXIT_MARKER="[Leeroy exited:"
tm() { tmux -L "$SOCKET" "$@"; }

cargo_() {
    if command -v rustc >/dev/null; then cargo "$@"; else nix develop -c cargo "$@"; fi
}

has_session() { tm has-session -t "$SESSION" 2>/dev/null; }

require_session() {
    has_session || { echo "no session; run: $0 start" >&2; exit 1; }
}

cmd=${1:-}
shift || true
case "$cmd" in
start)
    size=100x30
    if [[ ${1:-} =~ ^[0-9]+x[0-9]+$ ]]; then size=$1; shift; fi
    [[ ${1:-} == -- ]] && shift
    cargo_ build --quiet
    has_session && tm kill-session -t "$SESSION"
    : > target/tui.log
    # Wrapper keeps the pane alive after exit so crashes/exit codes stay visible.
    tm new-session -d -s "$SESSION" -x "${size%x*}" -y "${size#*x}" \
        -e LEEROY_LOG="$PWD/target/tui.log" -e RUST_LOG="${RUST_LOG:-debug}" \
        "$BIN $(printf '%q ' "$@"); echo; echo \"$EXIT_MARKER \$?]\"; sleep 86400"
    echo "started $SESSION ($size)"
    ;;
keys)
    require_session
    for k in "$@"; do tm send-keys -t "$SESSION" "$k"; done
    sleep 0.3 # let the app process + redraw
    ;;
capture)
    require_session
    if [[ ${1:-} == --ansi ]]; then tm capture-pane -p -e -t "$SESSION"; else tm capture-pane -p -t "$SESSION"; fi
    ;;
wait-for)
    require_session
    text=${1:?text required}
    deadline=$(( $(date +%s) + ${2:-5} ))
    until tm capture-pane -p -t "$SESSION" | grep -qF -- "$text"; do
        if (( $(date +%s) >= deadline )); then
            echo "timeout waiting for: $text" >&2
            tm capture-pane -p -t "$SESSION" >&2
            exit 1
        fi
        sleep 0.1
    done
    ;;
status)
    if ! has_session; then echo stopped; exit 0; fi
    line=$(tm capture-pane -p -t "$SESSION" | grep -F "$EXIT_MARKER" || true)
    if [[ -n $line ]]; then echo "exited (${line//[^0-9]/})"; else echo running; fi
    ;;
stop)
    has_session && tm kill-session -t "$SESSION"
    tm kill-server 2>/dev/null || true
    echo stopped
    ;;
*)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 1
    ;;
esac
