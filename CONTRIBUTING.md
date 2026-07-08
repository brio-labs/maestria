# Contributing to Maestria

Thank you for helping shape Maestria.

## Local workflow

- Keep changes confined to a package and its contracts.
- Preserve deterministic domain behavior; keep I/O and side effects in adapters/runtime.
- Keep files focused and boring.
- Prefer clear types over abstractions.

## Quality checks

Run before opening a PR:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --workspace --no-deps`
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`
- `python3 scripts/philosophy-check.py`
