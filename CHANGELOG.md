# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Docker container metadata for forwarded ports. When the host listener is
  Docker Desktop's `com.docker.backend`, a `docker-proxy`, or `vpnkit`,
  `portpeek` queries the local Docker Engine API over its Unix socket and shows
  the container name, ID, image, internal port, command, and Compose project.
- The optional JSON `container` object on Docker-forwarded listeners.
- `--no-command` also removes the command from container metadata.
- Colorized, aligned tables for human-readable output, with row separators,
  automatic color detection, and `--no-color`/`NO_COLOR` support.

### Fixed

- README badges, clone URLs, and issue-template links now point at the
  `port-peek` repository.
- Release archives no longer include the `unknown` vendor field in their
  filenames, for example `portpeek-x86_64-linux-gnu.tar.gz`.

## [0.1.0] - 2026-10-21

### Added

- Look up TCP listeners by port on macOS and Linux.
- Optional UDP lookup with `--udp`.
- PID, process name, executable path, full `argv`, shell-escaped command, and
  working directory for each listener.
- Local IPv4 and IPv6 binding addresses, including wildcard binds.
- Human-readable output grouped by process, plus `--json`.
- `--no-command` for safely sharing output that may contain secrets.
- Exit codes: `0` found, `1` not found, `2` usage/backend error.
- macOS backend using `libproc` and `sysctl(KERN_PROCARGS2)`.
- Linux backend using `NETLINK_SOCK_DIAG` and `/proc`.
- CI on macOS Apple Silicon/Intel and Linux x86_64/aarch64, MSRV check,
  Clippy, rustfmt, and `cargo-deny`.
- Tag-driven release workflow producing glibc and static musl archives with
  SHA-256 checksums.
- Dual MIT/Apache-2.0 licensing.

[Unreleased]: https://github.com/leorossi/port-peek/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/leorossi/port-peek/releases/tag/v0.1.0
