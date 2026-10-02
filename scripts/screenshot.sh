#!/usr/bin/env bash
# Render VHS tapes to PNG screenshots in target/screenshots/ (readme.tape:
# docs/screenshots/, committed and shown in the README).
#
#   scripts/screenshot.sh            run every tapes/*.tape
#   scripts/screenshot.sh help       run tapes/help.tape only
set -euo pipefail

cd "$(dirname "$0")/.."
# Isolate from your own setup: no LEEROY_* or *_PROXY env vars; each tape gets a fresh
# config at target/vhs/config.toml (set in tapes/_settings.tape).
# Exported names from `export -p` (Nix's non-interactive bash has no compgen).
shopt -s nocasematch
while read -r _ _ decl; do
    var=${decl%%=*}
    if [[ $var =~ ^[a-z_][a-z0-9_]*$ && $var =~ ^(leeroy_|(https?|all|no)_proxy$) ]]; then
        unset "$var"
    fi
done < <(export -p)
shopt -u nocasematch

run() {
    if command -v vhs >/dev/null && command -v rustc >/dev/null; then "$@"; else nix develop -c "$@"; fi
}

run cargo build --quiet
mkdir -p target/screenshots

# Tapes connect to a local fake Jenkins (http://127.0.0.1:8099), never the network.
# Own process group (setsid), so cleanup kills it and its children, nothing else.
setsid bash -c "$(declare -f run); run python3 scripts/fake-jenkins.py --port 8099" \
    > target/fake-jenkins.log 2>&1 &
fake_pgid=$!
trap 'kill -- "-$fake_pgid" 2>/dev/null || true' EXIT
for _ in $(seq 50); do grep -q 'fake Jenkins' target/fake-jenkins.log 2>/dev/null && break; sleep 0.1; done

if [[ $# -gt 0 ]]; then
    tapes=("${@/#/tapes/}")
    tapes=("${tapes[@]/%/.tape}")
else
    tapes=(tapes/*.tape)
fi

for tape in "${tapes[@]}"; do
    [[ $(basename "$tape") == _* ]] && continue # shared includes
    echo "==> $tape"
    # vhs can exit 0 without writing anything (see flake.nix), so delete the
    # expected screenshots first and check they exist afterwards.
    mapfile -t shots < <(sed -n 's/^Screenshot "\{0,1\}\([^"]*\)"\{0,1\}$/\1/p' "$tape")
    rm -f "${shots[@]}"
    for shot in "${shots[@]}"; do mkdir -p "$(dirname "$shot")"; done
    rm -rf target/vhs && mkdir -p target/vhs
    run vhs --quiet "$tape"
    for shot in "${shots[@]}"; do
        [[ -s $shot ]] || { echo "error: $tape did not produce $shot" >&2; exit 1; }
        echo "$shot"
    done
done
