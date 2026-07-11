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
- Prefer clear, boring abstractions with explicit contracts.
- Prefer stable, ordered types for deterministic state snapshots.

The first durable workflow is policy-scoped local indexing:

```bash
maestria init --instance-dir .maestria-dev --read-root ~/Projects --read-root ~/Notes
maestria index --instance-dir .maestria-dev ~/Projects/example --recursive
maestria search --instance-dir .maestria-dev "source-grounded phrase"
maestria open-evidence --instance-dir .maestria-dev --evidence-id 123
```

The instance manifest records approved read roots and sensitive-path exclusions.

## Documentation map

- `docs/PHILOSOPHY.md` — repository doctrine
- `docs/SPECS.md` — invariant ledger
- [`docs/architecture/`](./docs/architecture/) — architecture books
- `docs/first-pr-guide.md` — contributor onboarding

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

See [CONTRIBUTING.md](./CONTRIBUTING.md), [docs/first-pr-guide.md](./docs/first-pr-guide.md), and
[docs/PHILOSOPHY.md](./docs/PHILOSOPHY.md) for branch and review expectations.
