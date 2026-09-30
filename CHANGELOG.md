# Changelog

All notable changes to Leeroy are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and Leeroy uses
[semantic versioning](https://semver.org/spec/v2.0.0.html). While the version
is `0.x`, a minor bump may contain breaking changes (config format, key
bindings).

A release's section is used as its GitHub release notes.

## [Unreleased]

### Changed

- `b` now re-runs a build with the same parameters, and `B` starts a new
  build or pipeline run with the default parameters; both ask first.
  - Jobs tab: `b` re-runs the job's latest build, `B` builds it anew.
  - Builds tab, build view, console: `b` re-runs that build, `B` builds its
    job anew.
  - Pipelines and Pipeline runs tabs: `B` starts a new pipeline run.
  - Run view: `b` re-runs the selected step in the same run (through the
    Build Pipeline plugin when a view exists), `B` starts a new pipeline run.

## [0.4.1] - 2026-09-29

### Added

- The help popup shows Leeroy's version and build time.

## [0.4.0] - 2026-09-29

### Added

- Absolute timestamps: `t` switches when builds started between "17m ago"
  and the local date and time, in the Builds, Pipelines and Pipeline runs
  tabs, the run view and the build view. The choice is saved as the new
  `ui.timestamps` setting (also in the settings view, or
  `LEEROY_UI_TIMESTAMPS`).

### Changed

- Builds tab: paging down no longer waits at the end of what's loaded. The
  next three screenfuls load in the background (still one request) when you
  get within a screenful of the end, and a Down/PgDn at the end is carried
  out once they arrive. Refreshes reload only down to just below the
  selection, so they stay small after you scroll back up.

## [0.3.3] - 2026-09-29

### Changed

- Promotions: Enter only takes the steps selected with Space; the highlighted
  one is no longer taken when none is selected.
- The bottom bar shows `o` (open in browser) wherever it works, and drops
  hints that don't fit whole instead of cutting them off mid-word.

### Fixed

- The Jobs and Builds tabs were sluggish on big instances, most of all the
  Builds tab while filtering (66 ms per keypress with 100k builds, now 0.2 ms).

## [0.3.2] - 2026-09-29

### Fixed

- Promotions through the Build Pipeline plugin failed on current Jenkins,
  which loads the plugin's trigger from a script instead of writing it into the
  view page; Leeroy then fell back to starting the job directly, so the build
  wasn't part of the run. Build Pipeline views inside folders are found too, and
  a promotion that does fall back is shown as a warning.
- A step of a pipeline started by hand (or by such a fallback) showed up as a
  pipeline of its own when it triggered further jobs. Jobs the configuration
  says another job triggers are now always steps.

## [0.3.1] - 2026-09-27

### Added

- The run view shows the pipeline's parts a run never reached, dimmed, where
  they would have run (disabled jobs marked): this explains a partial run whose
  builds all succeeded.

### Changed

- In the run view, `b` is labelled "new run".

### Fixed

- The Pipelines and Pipeline runs lists were sluggish on big instances: every
  keypress and frame rebuilt all pipelines and runs. They're now worked out
  once per fetch and only the rows on screen are drawn (about 150× faster).
- A promotion just taken (build still queued) is no longer offered again, so it
  can't be started twice.

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

[Unreleased]: https://github.com/hbro/leeroy/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/hbro/leeroy/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/hbro/leeroy/compare/v0.3.3...v0.4.0
[0.3.3]: https://github.com/hbro/leeroy/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/hbro/leeroy/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/hbro/leeroy/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/hbro/leeroy/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/hbro/leeroy/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/hbro/leeroy/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/hbro/leeroy/releases/tag/v0.1.0
