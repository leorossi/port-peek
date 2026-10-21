# Contributing

Thanks for taking the time to improve `portpeek`.

## Development setup

- Rust 1.85 or newer (`rustup` is recommended)
- Python 3 for the integration tests
- macOS or Linux for running the backend tests

```console
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
$ cargo test
```

The Linux backend can be type-checked from macOS:

```console
$ rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
$ cargo check --target x86_64-unknown-linux-gnu
$ cargo check --target aarch64-unknown-linux-gnu
```

## What to keep in mind

- Do not shell out to `lsof`, `netstat`, `ss`, or `ps`. The point of this
  project is to use kernel interfaces directly.
- Keep platform-specific code inside `src/platform/`.
- Treat unreadable processes as normal: skip them or return a listener with
  `pid: None`; never fail the entire lookup because one process is protected.
- The command line can contain secrets. Preserve `--no-command` behaviour in any
  new output format.
- Add or update tests for behaviour changes. Integration tests spawn real
  sockets, so they must clean up child processes.

## Pull requests

1. Open an issue for larger changes so the approach can be discussed.
2. Keep commits focused and use a clear imperative subject.
3. Make sure `cargo fmt --check`, Clippy, and the test suite pass.
4. Update `CHANGELOG.md` under `[Unreleased]` for user-visible changes.

By contributing, you agree that your contribution is dual licensed under
MIT OR Apache-2.0, as described in the README.

## Releasing

1. Update `version` in `Cargo.toml`.
2. Move the `[Unreleased]` entries in `CHANGELOG.md` under the new version and
   add comparison links.
3. Run the full test suite, then commit.
4. Create and push a tag that exactly matches the package version:

   ```console
   $ git tag v0.1.0
   $ git push origin v0.1.0
   ```

The `Release` workflow verifies the tag, builds macOS and Linux archives (glibc
and static musl), writes SHA-256 checksums, and creates the GitHub release.

To publish to crates.io automatically, set the repository variable
`PUBLISH_CRATES_IO=true` and the `CARGO_REGISTRY_TOKEN` secret. Otherwise run
`cargo publish` manually after the release.
