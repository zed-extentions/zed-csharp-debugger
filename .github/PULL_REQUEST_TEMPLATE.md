## Summary

<!-- What does this change do, and why? -->

## Related requirements

<!-- FR/NFR IDs from REQUIREMENTS.md affected or added by this change -->

## Verification

- [ ] `cargo test` passes
- [ ] `cargo fmt --all -- --check` clean
- [ ] `cargo clippy --all-targets -- -D warnings` clean
- [ ] `cargo build --target wasm32-wasip2 --release` succeeds
- [ ] Documentation updated (README / REQUIREMENTS) if behavior changed
