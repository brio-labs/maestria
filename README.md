# Maestria

Maestria is a local-first, source-grounded second-brain runtime for AI agents.

## Architecture

| Crate | Layer | Description |
|-------|-------|-------------|
| `maestria-domain` | Kernel | Deterministic domain types, events, and transitions |
| `maestria-governance` | Governance | Decision gates and runtime adapter contracts |

## Invariants and Workflow

- Domain and governance are kept side-effect free.
- All side effects are represented as typed intentions (effects).
- Policy and mechanism are separated by trait boundaries.
- Every change is validated through local checks and repository checks.

## Local workflow

- Keep changes scoped to one crate unless cross-crate coupling is required.
- Preserve deterministic behavior at the domain layer.
- Keep side effects in adapters and runtime.
- Prefer clear, boring abstractions.
- Prefer stable, ordered types for deterministic state snapshots.

## Quick start

```bash
# 1) Formatting
cargo fmt --all -- --check

# 2) Linting
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 3) Tests
cargo test --workspace --all-targets --all-features

# 4) Documentation
cargo doc --workspace --no-deps --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features

# 5) Architecture guardrails
python3 scripts/philosophy-check.py
```

## Contributing

See [CONTRIBUTING.md](./CONTRIBUTING.md) for branch strategy, commit conventions,
and quality checks.