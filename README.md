# portpeek

[![CI](https://github.com/leorossi/portpeek/actions/workflows/ci.yml/badge.svg)](https://github.com/leorossi/portpeek/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/leorossi/portpeek?display_name=tag)](https://github.com/leorossi/portpeek/releases)
[![crates.io](https://img.shields.io/crates/v/portpeek.svg)](https://crates.io/crates/portpeek)
[![Downloads](https://img.shields.io/crates/d/portpeek.svg)](https://crates.io/crates/portpeek)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.85%2B-orange.svg)](Cargo.toml)
[![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)](#platform-support)

**Which process is listening on this port, and what is it running?**

`portpeek` shows the process ID, executable, **full command line**, working
directory, and local IP address of every listener on a port. It calls kernel
interfaces directly — it never parses the output of `lsof`, `netstat`, or `ss`.
For ports forwarded by Docker, it also identifies the container behind the
forwarder.

```console
$ portpeek 8080
TCP 8080 · 1 process

PID 4242 · node
  exe        /opt/homebrew/bin/node
  listening  127.0.0.1:8080, [::1]:8080
  cmd        node server.js --port 8080 --inspect
  cwd        /Users/leo/src/api
```

## Features

- PID, process name, and full executable path
- Full argument vector (`argv`) as reported by the kernel; non-UTF-8 bytes are
  replaced lossily
- Working directory
- IPv4 and IPv6 bindings, including `0.0.0.0` and `::`
- Multiple listeners on the same port, grouped by process
- Docker container name, ID, image, and internal port for forwarded ports
  (read from the local Docker Engine API)
- TCP by default, UDP with `--udp`
- Human-readable cards or stable JSON
- Linux static (musl) and glibc binaries, x86_64 and aarch64
- macOS Apple Silicon and Intel binaries
- No setuid binary, daemon, or background service

## Installation

### Build and run from a checkout

From the directory that contains `Cargo.toml`:

```console
$ cargo build
$ ./target/debug/portpeek 8080
```

Cargo writes the debug binary to `target/debug/portpeek`. You can also run it
from inside that directory:

```console
$ cd target/debug
$ ./portpeek 8080
```

To compile and run in one step:

```console
$ cargo run -- 8080
```

For an optimized binary:

```console
$ cargo build --release --locked
$ ./target/release/portpeek 8080
```

or:

```console
$ cargo run --release -- 8080
```

Pass portpeek options after `--`, for example:

```console
$ cargo run -- 53 --udp
$ cargo run -- 8080 --json
```

If you do not have the source yet:

```console
$ git clone https://github.com/leorossi/portpeek.git
$ cd portpeek
$ cargo build --release
```

### Install from a checkout

The simplest system install uses Cargo:

```console
$ cargo install --path . --locked
```

This installs `portpeek` into Cargo's binary directory (`~/.cargo/bin` by
default). Ensure that directory is on your `PATH`, then run:

```console
$ portpeek 8080
```

After changing the source, reinstall with:

```console
$ cargo install --path . --locked --force
```

Uninstall it with:

```console
$ cargo uninstall portpeek
```

To install the optimized binary directly instead:

```console
$ cargo build --release --locked
$ sudo install -d /usr/local/bin
$ sudo install -m 0755 target/release/portpeek /usr/local/bin/portpeek
```

Without `sudo`, install it for your user:

```console
$ mkdir -p "$HOME/.local/bin"
$ install -m 0755 target/release/portpeek "$HOME/.local/bin/portpeek"
```

Make sure `$HOME/.local/bin` is on your `PATH`. Remove the binary with
`sudo rm /usr/local/bin/portpeek` or `rm "$HOME/.local/bin/portpeek"`.

To inspect processes owned by other users, build first and then run the installed
binary with `sudo`. If it is in `~/.cargo/bin`, use the full path because `sudo`
may not include that directory in `PATH`:

```console
$ sudo "$HOME/.cargo/bin/portpeek" 443
```

### Prebuilt binary

Download the archive for your platform from the
[latest release](https://github.com/leorossi/portpeek/releases/latest), then put
`portpeek` somewhere on your `PATH`.

```console
$ tar xzf portpeek-aarch64-apple-darwin.tar.gz
$ sudo mv portpeek /usr/local/bin/
```

Linux glibc archives are built on Ubuntu 24.04. On an older distribution, use the
static musl archive instead.

### Install from crates.io

After the crate is published:

```console
$ cargo install portpeek
```

## Usage

```console
$ portpeek [OPTIONS] <PORT>
```

| Option | Description |
| --- | --- |
| `--udp` | Inspect UDP sockets instead of TCP |
| `--json` | Print machine-readable JSON |
| `--no-command` | Omit `argv`/`command`, including Docker container commands |
| `-h, --help` | Show help |
| `-V, --version` | Show version |

### Examples

```console
$ portpeek 3000
$ portpeek 5432 --json
$ portpeek 53 --udp
$ sudo portpeek 443
```

For UDP, `portpeek` reports unconnected sockets bound to the port (the
equivalent of `ss -u -l`). Connected UDP sockets are not servers and are
omitted.

The command is shown with POSIX shell escaping so it can be copied into a
shell. The original quoting cannot be recovered from `argv`; the `argv` array in
JSON is the most faithful form available.

### Docker-forwarded ports

Docker Desktop on macOS forwards published ports through `com.docker.backend`;
on Linux, `docker-proxy` usually owns the host socket. `portpeek` recognizes
these forwarders, asks the local Docker Engine API which container publishes the
port, and adds the container metadata to the report:

```console
$ portpeek 8000
TCP 8000 · 1 process

PID 98560 · com.docker.backend
  exe        /Applications/Docker.app/Contents/MacOS/com.docker.backend
  container  telemetry-demo-app (9f8cac51baba)
  image      telemetry-demo-app
  internal   8000/tcp
  compose    2026-10-14-laravel-telemetry-talk
  listening  [::]:8000
  cmd        entrypoint.sh php artisan serve --host=0.0.0.0 --port=8000
  cwd        /Users/leo/Library/Containers/com.docker.docker/Data
```

The API request is made directly over a Unix socket; no `docker` command is
executed and no command output is parsed. `portpeek` tries `DOCKER_HOST` when it
is a `unix://` address, `/var/run/docker.sock`,
`$XDG_RUNTIME_DIR/docker.sock`, `~/.docker/run/docker.sock`, and
`~/.colima/default/docker.sock`.

This lookup is automatic and best-effort. If the socket is unavailable or the
daemon cannot be reached, the normal forwarder details are shown. `--no-command`
also removes the container command from human and JSON output.

### JSON

```json
{
  "port": 8080,
  "protocol": "tcp",
  "listeners": [
    {
      "pid": 4242,
      "process": "node",
      "exe": "/opt/homebrew/bin/node",
      "argv": ["node", "server.js", "--port", "8080"],
      "command": "node server.js --port 8080",
      "cwd": "/Users/leo/src/api",
      "protocol": "tcp",
      "ip": "127.0.0.1",
      "port": 8080
    }
  ]
}
```

One object is emitted per listening socket. If a socket is visible but its owner
cannot be inspected, `pid` and the process fields are `null` and the entry is
still included.

When a listener is a Docker forwarder, the object also contains a `container`
object. Its presence is optional; `--no-command` removes `command` from it:

```json
{
  "pid": 98560,
  "process": "com.docker.backend",
  "protocol": "tcp",
  "ip": "::",
  "port": 8000,
  "container": {
    "id": "9f8cac51baba9eede78bf7fd180ba5d9d655b765e021b2b180f2318ad7c64fd5",
    "name": "telemetry-demo-app",
    "image": "telemetry-demo-app",
    "internal_port": 8000,
    "protocol": "tcp",
    "command": "entrypoint.sh php artisan serve --host=0.0.0.0 --port=8000",
    "compose_project": "2026-10-14-laravel-telemetry-talk"
  }
}
```

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | At least one listener was found |
| `1` | No listener was found |
| `2` | Invalid usage or a backend error |

## Permissions

Kernel metadata is protected per user:

- **Without privileges**, only processes owned by you (or otherwise readable)
  are fully resolved. On macOS, sockets belonging to other users may not be
  visible at all; on Linux, the socket is usually visible but its owner is not.
- **With `sudo`**, all normal processes are resolved. A few SIP-protected macOS
  system processes may still hide some details.
- Docker metadata needs read access to the Docker Engine Unix socket. On Linux
  that usually means root or membership in the `docker` group; without access,
  only the forwarder is reported.
- `portpeek` is **not** setuid and does not require a daemon.

If a socket is visible but its owner is not, it is shown with `PID ?` and a note
to re-run with `sudo`. On macOS, historical behaviour is the same: `/bin/ps` is
setuid root for this reason.

## Platform support

| Platform | Architecture | Status |
| --- | --- | --- |
| macOS | Apple Silicon (`aarch64`) | Supported |
| macOS | Intel (`x86_64`) | Supported |
| Linux (glibc) | `x86_64`, `aarch64` | Supported |
| Linux (musl, static) | `x86_64`, `aarch64` | Supported |
| Windows | — | Not yet (the backend is isolated for future `GetExtendedTcpTable` support) |

## How it works

`portpeek` reads the kernel's own data structures.

### macOS

- `proc_listallpids` to enumerate processes
- `proc_pidinfo(PROC_PIDLISTFDS)` and `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)`
  to find sockets in `TSI_S_LISTEN` state and decode their local address/port
- `proc_pidpath` for the executable
- `proc_pidinfo(PROC_PIDVNODEPATHINFO)` for the working directory
- `sysctl(KERN_PROCARGS2)` for the argument vector

Apple labels `libproc` as a private interface, but it is the stable interface
used by `lsof` and `ps`. The layout structs are isolated in
`src/platform/macos.rs` and exercised by CI on both architectures.

### Linux

- `NETLINK_SOCK_DIAG` with `SOCK_DIAG_BY_FAMILY` dumps for IPv4 and IPv6
- `inet_diag_msg` for state, local address, port, and socket inode
- `/proc/<pid>/fd/*` symlinks to map socket inodes to PIDs
- `/proc/<pid>/exe`, `/proc/<pid>/cwd`, and `/proc/<pid>/cmdline` for process
  metadata

No external command is executed.

### Docker metadata

- A listener whose executable is Docker Desktop's `com.docker.backend`, a
  `docker-proxy`, or `vpnkit` is treated as a port forwarder.
- `portpeek` sends `GET /containers/json?all=0` to the Docker Engine API over a
  local Unix socket and decodes the HTTP/chunked response directly.
- Published ports are matched by host port, transport protocol, and listener
  address (an exact address wins over a wildcard).

## Limitations

- **Command lines can contain secrets.** Tokens and passwords often appear in
  `argv`. Use `--no-command` before sharing output.
- Some daemons rewrite their `argv` (for example Postgres and nginx), so the
  displayed command can be a status string rather than the original launch
  command.
- Arguments that are not valid UTF-8 are converted lossily.
- If several processes inherit the same listening socket, the lowest PID is
  reported for that socket.
- **Docker:** on macOS the VM forwards ports through
  `com.docker.backend`/vpnkit; on Linux, a published port is usually held by
  `docker-proxy` in the host network namespace. `portpeek` enriches these
  forwarders with container metadata when the Docker API socket is readable.
  Without access to the socket, only the forwarder is shown.
- Linux containers with a separate PID namespace or `hidepid` may limit
  attribution; `portpeek` sees the namespace it runs in.

## Development

```console
$ cargo fmt --check
$ cargo clippy --all-targets -- -D warnings
$ cargo test
```

Cross-check the Linux backend without leaving macOS:

```console
$ rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
$ cargo check --target x86_64-unknown-linux-gnu
$ cargo check --target aarch64-unknown-linux-gnu
```

The integration tests spawn real TCP/UDP listeners (including a child process
with spaces and quotes in its arguments) and assert that PID, executable,
`argv`, cwd, and address all match.

## Contributing

Issues and pull requests are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for
the development workflow. Please report security problems privately as
described in [SECURITY.md](SECURITY.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.
