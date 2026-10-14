[`CLAUDE.md`](https://github.com/hbro/leeroy/blob/main/CLAUDE.md) describes the architecture and how to verify changes (tests,
snapshots, tmux, screenshots). CI (`.github/workflows/ci.yml`) runs rustfmt and
clippy, and the tests on Linux (x86_64 glibc and musl, ARM64), macOS and
Windows, for every push to `main` and every pull request.

## Documentation

These pages are the source of the [wiki](https://github.com/hbro/leeroy/wiki):
edit them in [`docs/`](https://github.com/hbro/leeroy/tree/main/docs), never
in the wiki itself. `.github/workflows/wiki.yml` copies `docs/` (pages and
screenshots) to the wiki repo for every release tag, and when run by hand
(Actions → Wiki → Run workflow) for fixes between releases; it overwrites
whatever was changed in the wiki. Links between pages are written as
`Page.md` so they work in the repo too; the workflow drops the `.md`.

The wiki has one version: the latest release (the footer names it). Every
version's own docs are `docs/` at its tag
(`https://github.com/hbro/leeroy/tree/vX.Y.Z/docs`; up to 0.5.2 they were in
the README) and in its release archive; `CHANGELOG.md` says what changed when.

## Releasing

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
