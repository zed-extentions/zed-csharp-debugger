# Contributing to zed-csharp-debugger

Thanks for your interest in contributing!

## Development setup

- Rust (stable) with the `wasm32-wasip2` target: `rustup target add wasm32-wasip2`
- Verify before every commit (CI enforces the same):
  ```bash
  cargo test
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  cargo build --target wasm32-wasip2 --release
  ```

## How changes are made here

1. Every change traces to a requirement ID (FR/NFR) in [REQUIREMENTS.md](REQUIREMENTS.md) —
   new features add the requirement first.
2. Keep pure modules free of `zed_extension_api` dependencies; Zed API touchpoints stay in
   `lib.rs` (see [`.rules`](.rules) and [PLAN.md](PLAN.md)).
3. Open a pull request against `main`; CI must pass. Reviews are welcome from anyone.

Working rules for humans and AI coding agents alike live in [`.rules`](.rules).
