# portpeek implementation plan

`portpeek` answers: **which process is listening on this port, and what is it running?**

It reports PID, executable name/path, full argument vector, working directory, and the
local IP/port it is listening on. It calls kernel interfaces directly (macOS `libproc` +
`sysctl`; Linux `NETLINK_SOCK_DIAG` + `/proc`) and never parses `lsof`, `netstat`, or `ss`
output.

Legend: `[ ]` pending · `[x]` done and verified.

## 0. Toolchain and repository scaffold

- [x] Find/install a Rust toolchain (`rustup`, stable 1.99.1 on this machine)
- [x] Add Linux cross-check targets (`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`)
- [x] Create `Cargo.toml` with binary/library layout and package metadata
- [x] Add `rust-toolchain.toml`, `.gitignore`, `.editorconfig`, `clippy.toml`
- [x] Add `LICENSE-MIT` and `LICENSE-APACHE` (dual license)
- [x] Initialize the git repository on `main`
- [x] Verify `cargo metadata`, `cargo check`, and `cargo package`

## 1. Shared model, errors, CLI

- [x] Define `Protocol`, `Listener`, and `Report` in `src/model.rs`
- [x] Implement normalization: stable sort, deduplication, unresolved-owner last
- [x] Implement POSIX-shell-safe command rendering from `argv`
- [x] Add `src/error.rs` with a small `Error`/`Result` surface
- [x] Add `src/cli.rs` (`PORT`, `--udp`, `--json`, `--no-command`)
- [x] Wire `src/lib.rs` (`find_listeners`) and `src/main.rs` (exit codes 0/1/2)
- [x] Unit-test CLI parsing, normalization, and shell quoting

## 2. macOS backend (`src/platform/macos.rs`)

- [x] Enumerate PIDs with `proc_listallpids`
- [x] List socket fds with `PROC_PIDLISTFDS` / `proc_pidfdinfo`
- [x] Match TCP `TSI_S_LISTEN` sockets and decode IPv4/IPv6 local address + port
- [x] Match bound UDP sockets when `--udp` is used
- [x] Resolve executable path (`proc_pidpath`) and name
- [x] Resolve working directory (`PROC_PIDVNODEPATHINFO` → `pvi_cdir.vip_path`)
- [x] Resolve full argv via `sysctl(KERN_PROCARGS2)`, including padding parsing
- [x] Handle permission errors, vanished processes, and unreadable argv gracefully
- [x] Add SDK-layout assertions for all mirrored C structs
- [x] Manual test: find a locally spawned TCP listener and inspect all fields
- [x] Manual test: find a locally spawned UDP socket and an IPv6 listener

## 3. Linux backend (`src/platform/linux.rs`)

- [x] Open and bind a `NETLINK_SOCK_DIAG` socket
- [x] Send `SOCK_DIAG_BY_FAMILY` dumps for IPv4 and IPv6
- [x] Parse `inet_diag_msg`: state, local address, port, inode, uid
- [x] Filter TCP `TCP_LISTEN` and unconnected UDP (`TCP_CLOSE`)
- [x] Build the inode → PID index by reading `/proc/<pid>/fd` symlinks
- [x] Resolve `/proc/<pid>/exe`, `/proc/<pid>/cwd`, `/proc/<pid>/comm`
- [x] Resolve `/proc/<pid>/cmdline`, preserving empty arguments
- [x] Handle `EPERM`, `ENOENT`, `hidepid`, and processes exiting mid-scan
- [x] Cross-check compilation for `x86_64` and `aarch64` Linux
- [x] Runtime-test on Linux aarch64 (Ubuntu 26.04 VM), including unprivileged vs `sudo`
- [x] Runtime-test UDP and IPv6 on Linux
- [x] Verify the static musl release build

## 4. Output

- [x] Human output: colorized per-process tables with grouped addresses, full
      command, and cwd
- [x] Show a clear hint when a socket owner cannot be resolved
- [x] JSON output: stable schema, one entry per listening socket
- [x] `--no-command` removes command/argv from both human and JSON output
- [x] `--no-color`, `NO_COLOR`, and non-TTY detection for color output
- [x] Correct exit codes: 0 found, 1 no listener, 2 error
- [x] Handle broken pipes without panicking

## 5. Test suite

- [x] Unit tests for argument parsing, quoting, and report normalization
- [x] Integration: current test process TCP listener on `127.0.0.1`
- [x] Integration: IPv6 listener on `[::1]` (skip if unavailable)
- [x] Integration: UDP socket with `--udp`
- [x] Integration: connected UDP client is not treated as a listener
- [x] Integration: child process with spaces/quotes in argv, custom cwd
- [x] Integration: unused port returns an empty report
- [x] CLI tests: invalid port exit 2, no listener exit 1, JSON + `--no-command`
- [x] `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`
- [x] MSRV check with Rust 1.85 (full test suite, not just `check`)

## 6. GitHub-ready documentation and automation

- [x] README with badges, install, usage, JSON, permissions, and Docker caveat
- [x] `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`
- [x] `deny.toml` (licenses/advisories) and Dependabot config
- [x] `.github/workflows/ci.yml` (fmt, clippy, tests on 4 runners, MSRV, deny)
- [x] `.github/workflows/release.yml` (tagged binaries, checksums, GitHub Release)
- [x] Issue templates and pull-request template
- [x] Final review, `cargo package`, initial git commit

## 7. Docker-forwarded ports

- [x] Recognize Docker Desktop (`com.docker.backend`), `docker-proxy`, and
      `vpnkit` host listeners
- [x] Query the Docker Engine API over a Unix socket without executing `docker`
- [x] Decode HTTP chunked responses and parse the container list
- [x] Match published ports by host port, protocol, and listener address
- [x] Add optional container ID, name, image, internal port, command, and
      Compose project to the model, human output, and JSON
- [x] Make `--no-command` remove Docker container commands as well
- [x] Unit-test forwarder detection, HTTP/chunked parsing, matching, and output
- [x] Manual test against a real Docker Desktop container on port 8000

## 8. Final verification

- [x] All tests pass locally on macOS (33 unit, 10 integration)
- [x] Linux tests passed on Ubuntu 26.04 aarch64 before the Docker addition;
      the Docker code cross-compiles and is covered by platform-independent
      unit tests
- [x] Linux code compiles and Clippy-clean for `x86_64` and `aarch64`
- [x] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] `cargo fmt --check`
- [x] `cargo deny check`
- [x] `actionlint` on both workflows; YAML templates parse
- [x] Manual smoke test with a real server, `--json`, `--udp`, IPv6, and `sudo`
- [x] Release archive smoke test (static musl binary and glibc archive)
- [x] README commands and badge URLs checked
- [x] Working tree clean, single initial commit

## Verification notes (2026-10-21)

- **macOS 15, arm64:** `cargo test`, `cargo clippy`, `cargo fmt --check`,
  `cargo package`, MSRV 1.85 test suite, manual TCP/UDP/IPv6 lookups, and a
  Docker Desktop lookup on port 8000 showing the container name, image,
  internal port, Compose project, and container command. Colors were verified
  through a pseudo-terminal, and disabled for piped output, `NO_COLOR`, and
  `--no-color`.
- **Linux aarch64 (Ubuntu 26.04 Lima VM):** full test suite, Clippy, fmt,
  static musl build, glibc release archive, and unprivileged vs `sudo`
  attribution. The Docker client is platform-independent and cross-compiles for
  both Linux targets.
- **Linux x86_64:** cross-compiled and Clippy-checked on macOS; the native
  x86_64 runner in CI runs the full suite.
- Release archives contain `portpeek`, `README.md`, and both license files.
- The GitHub Actions runner labels (`macos-15`, `macos-15-intel`,
  `ubuntu-latest`, `ubuntu-24.04-arm`) match the current runner-images labels.
