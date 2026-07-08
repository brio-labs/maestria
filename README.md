# Maestria

Maestria is a local-first, source-grounded second-brain runtime for AI agents.

## Bootstrap Status

The repository has been initialized with an architecture-aligned workspace skeleton:

- `crates/kernel/maestria-domain`
- `crates/kernel/maestria-governance`
- Governance and bootstrap documentation under `docs/`
- Contract and replay scaffolding directories under `tests/`

## Quick start

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --workspace --no-deps`
- `python3 scripts/philosophy-check.py`
