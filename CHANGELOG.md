# Changelog

All notable changes to Leeroy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Leeroy uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). While the version
is `0.x`, a minor bump may contain breaking changes (config format, key
bindings).

A release's section is used as its GitHub release notes.

## [Unreleased]

### Added

- The run view shows the pipeline's parts a run never reached, dimmed, where
  they would have run (disabled jobs marked): this explains a partial run whose
  builds all succeeded.

## [0.3.0] - 2026-09-27

### Added

- Pipelines tab: every job that triggers others, named by its jobs' common
  prefix, with the status of its latest run (success, partial, unstable,
  aborted, failure). Relations come from both the job configuration and the
  builds' upstream causes, so Jenkinsfile `build job:` steps count too.
- Pipeline runs tab: every run of every pipeline, newest first.
- Run view (Enter in either tab): the run's builds as a tree or (`v`) as
  stacked boxes, older/newer runs with `←`/`→`, Enter opens a build.
- Build view: the stages of Pipeline builds, when Jenkins has the Pipeline Stage
  View plugin.
- `b` on the Pipelines tab or in the run view: start a new run of the pipeline
  (after a y/n prompt, with default parameter values); on the Jobs and Builds
  tabs, in the build view and the console: start a build of that job.
- `p` in the run view: promote, i.e. take the run's manual steps (e.g. the Build
  Pipeline plugin's), several at once; the new builds join the run.
- `o`: open what's on screen in Jenkins' web UI (without URL credentials).
- `i`: an overlay about the connected Jenkins instance (version, user, nodes,
  executors, queue, quieting down) and how Leeroy reaches it.

### Changed

- The help popup lists the current view's keys under "Navigation"; arrow,
  Home/End and PgUp/PgDn keys are only listed there, not in the bottom bar.

## [0.2.0] - 2026-09-26

### Added

- Light theme, and an `auto` theme (the default) that picks dark or light from
  the terminal's background colour. Setting `ui.theme` (`LEEROY_UI_THEME`),
  switchable live in the settings view.

### Removed

- The `s` shortcut for the settings: open them with `0`, their tab key.

## [0.1.1] - 2026-09-26

### Added

- Static Linux builds (musl) for x86_64 and ARM64, which run on any
  distribution, next to the glibc builds.

### Changed

- Release archives are named without the vendor part of the target, and the
  Linux glibc builds say `glibc` instead of `gnu`: e.g.
  `leeroy-v0.1.1-x86_64-linux-glibc.tar.gz`, `…-aarch64-darwin.tar.gz`,
  `…-x86_64-windows-msvc.zip`.

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
- Prebuilt binaries for Linux (x86_64, ARM64), macOS (Apple Silicon, Intel)
  and Windows (x86_64).

[Unreleased]: https://github.com/hbro/leeroy/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/hbro/leeroy/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/hbro/leeroy/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/hbro/leeroy/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/hbro/leeroy/releases/tag/v0.1.0
