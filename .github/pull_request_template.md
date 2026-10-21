## Summary

<!-- What changed and why? -->

## Testing

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `cargo test`
- [ ] Tested on: <!-- macOS arm64 / macOS x86_64 / Linux x86_64 / Linux arm64 -->

## Checklist

- [ ] The change uses kernel interfaces rather than parsing external command output.
- [ ] `--no-command` still removes `argv` and the rendered command.
- [ ] User-visible changes are listed in `CHANGELOG.md`.
- [ ] New behaviour has tests, or the reason it cannot be tested is explained.
