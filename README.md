# Leeroy – a TUI for Jenkins

> *All right, chums! Let's do this! LEEROOOOOOOOOOOOOY! JEEEENKIIINS!*
>
> — Leeroy Jenkins

## Overview

The jobs of an instance, with their last result and running builds:

![Jobs tab: every job with its last result](docs/screenshots/jobs.png)

The build history of all jobs, newest first:

![Builds tab: recent builds across all jobs](docs/screenshots/builds.png)

A build's details, here a running one with its progress:

![Build details of a running build](docs/screenshots/build.png)

Pipelines (jobs that trigger other jobs) with the status of their latest run:

![Pipelines tab: each pipeline with its latest run's status](docs/screenshots/pipelines.png)

A run: the builds it triggered, as a tree or (with `v`, shown here) as boxes:

![Run view: a pipeline's latest run as boxes](docs/screenshots/run.png)

## Installing

Download the archive for your platform from the
[releases](https://github.com/hbro/leeroy/releases) (`SHA256SUMS` lists the
checksums) and put the `leeroy` binary on your `PATH`:

| Platform | Archive |
| --- | --- |
| Linux x86_64 | `leeroy-vX.Y.Z-x86_64-linux-musl.tar.gz` or `…-x86_64-linux-glibc.tar.gz` |
| Linux ARM64 | `leeroy-vX.Y.Z-aarch64-linux-musl.tar.gz` or `…-aarch64-linux-glibc.tar.gz` |
| macOS Apple Silicon | `leeroy-vX.Y.Z-aarch64-darwin.tar.gz` |
| macOS Intel | `leeroy-vX.Y.Z-x86_64-darwin.tar.gz` |
| Windows x86_64 | `leeroy-vX.Y.Z-x86_64-windows-msvc.zip` |

On Linux, the `musl` builds are static and run on any distribution (Alpine
included); the `glibc` builds link against glibc 2.35 or newer. macOS binaries
aren't signed: clear the quarantine flag with
`xattr -d com.apple.quarantine leeroy`.

Or build from source: `cargo install --git https://github.com/hbro/leeroy`.
`leeroy --version` prints the version.

## Getting started

Run `leeroy`, open the settings with `0` and enter your Jenkins URL; add an
`Authorization` header if your instance needs a login (see
[Connecting](docs/Connecting.md)). `1`–`4` switch between the Jobs, Builds,
Pipelines and Pipeline runs tabs, and `h` or `?` lists every key.

## Documentation

The [wiki](https://github.com/hbro/leeroy/wiki) has the full documentation;
its source is in [`docs/`](docs/) (also included in the release archives):

- [Installing](docs/Installing.md)
- [Configuration](docs/Configuration.md): the config file, env vars, TLS, colours
- [Connecting](docs/Connecting.md): authentication headers, proxies
- [Using Leeroy](docs/Using-Leeroy.md): tabs and global keys
- [Jobs and Builds](docs/Jobs-and-Builds.md): job list, build details, console
  output, build history
- [Pipelines](docs/Pipelines.md): pipelines, their runs, promotions, re-running
- [Troubleshooting](docs/Troubleshooting.md): logs, debug output, common problems
- [Development](docs/Development.md): architecture, CI, releasing, these docs
