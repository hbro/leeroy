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
