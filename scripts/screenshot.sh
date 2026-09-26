#!/usr/bin/env bash
# Render VHS tapes to PNG screenshots in target/screenshots/.
#
#   scripts/screenshot.sh            run every tapes/*.tape
#   scripts/screenshot.sh help       run tapes/help.tape only
set -euo pipefail

cd "$(dirname "$0")/.."
# Isolate from your own setup: no LEEROY_* env vars; each tape gets a fresh
# config at target/vhs/config.toml (set in tapes/_settings.tape).
while read -r var; do unset "$var"; done < <(compgen -e | grep '^LEEROY_' || true)

run() {
    if command -v vhs >/dev/null && command -v rustc >/dev/null; then "$@"; else nix develop -c "$@"; fi
}

run cargo build --quiet
mkdir -p target/screenshots

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
    rm -rf target/vhs && mkdir -p target/vhs
    run vhs --quiet "$tape"
    for shot in "${shots[@]}"; do
        [[ -s $shot ]] || { echo "error: $tape did not produce $shot" >&2; exit 1; }
        echo "$shot"
    done
done
