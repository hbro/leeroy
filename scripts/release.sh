#!/usr/bin/env bash
# Prepare a release: set the version in Cargo.toml (and Cargo.lock), turn the
# changelog's [Unreleased] section into the new version's section, run the
# checks. Doesn't commit, tag or push; prints the commands for that.
#
#   scripts/release.sh 0.2.0
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v rustc > /dev/null; then
  exec nix develop -c "$0" "$@"
fi

version=${1:-}
if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "usage: $0 X.Y.Z[-pre]   (semantic version, no leading v)" >&2
  exit 2
fi
if [[ -n $(git status --porcelain) ]]; then
  echo "working tree not clean; commit or stash first" >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/v$version" > /dev/null; then
  echo "tag v$version already exists" >&2
  exit 1
fi
if ! grep -q '^## \[Unreleased\]' CHANGELOG.md; then
  echo "CHANGELOG.md has no '## [Unreleased]' section" >&2
  exit 1
fi
if [[ -z $(awk '/^## \[Unreleased\]/ { f = 1; next } f && /^## \[/ { exit } f && NF' CHANGELOG.md) ]]; then
  echo "the [Unreleased] section of CHANGELOG.md is empty" >&2
  exit 1
fi

previous=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" Cargo.toml
cargo update --offline --workspace --quiet

today=$(date +%Y-%m-%d)
repo=https://github.com/hbro/leeroy
# New version section under a fresh [Unreleased]; fix the links at the bottom.
awk -v v="$version" -v d="$today" -v repo="$repo" -v prev="$previous" '
  /^## \[Unreleased\]/ { print; print ""; print "## [" v "] - " d; next }
  /^\[Unreleased\]:/ {
    print "[Unreleased]: " repo "/compare/v" v "...HEAD"
    if (released_before) print "[" v "]: " repo "/compare/v" prev "...v" v
    else print "[" v "]: " repo "/releases/tag/v" v
    next
  }
  { print }
' released_before="$(git rev-parse -q --verify "refs/tags/v$previous" > /dev/null && echo 1)" \
  CHANGELOG.md >| CHANGELOG.md.new
mv CHANGELOG.md.new CHANGELOG.md

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --quiet

cat << MSG

Prepared v$version ($previous → $version). Review the diff, then:

  git commit -am "Release v$version"
  git tag -a v$version -m "Leeroy v$version"
  git push origin main v$version

The Release workflow builds the binaries and publishes the GitHub release.
MSG
