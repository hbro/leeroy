# Changelog

All notable changes to Leeroy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Leeroy uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). While the version
is `0.x`, a minor bump may contain breaking changes (config format, key
bindings).

A release's section is used as its GitHub release notes.

## [Unreleased]

## [0.1.0] - 2026-09-26

### Added

- Jobs tab: all jobs (folders flattened) with their last result, live filter.
- Builds tab: build history of all jobs, newest first, filterable, loaded one
  screenful at a time.
- Build view: result, timing, trigger, parameters, changes and a progress bar
  for running builds; step between builds with `←`/`→`, `Home`/`End`.
- Console view: tails a running build's output, with scrolling and follow/pause.
- Manual (`r`) and automatic (`R`, configurable interval) refresh, with the age
  of the data in the header.
- Settings tab: Jenkins URL, custom HTTP headers for authentication, proxy
  (HTTP/HTTPS/SOCKS, curl-style `*_proxy` fallback), skip TLS verification,
  refresh and quit confirmation; every setting can be overridden with a
  `LEEROY_*` environment variable.
- Config file in `$XDG_CONFIG_HOME/leeroy/config.toml`
  (`%APPDATA%\leeroy\config.toml` on Windows), or `--config` /
  `$LEEROY_CONFIG`.
- Prebuilt binaries for Linux (x86_64, ARM64; static musl and glibc), macOS
  (Apple Silicon, Intel) and Windows (x86_64).

[Unreleased]: https://github.com/hbro/leeroy/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hbro/leeroy/releases/tag/v0.1.0
