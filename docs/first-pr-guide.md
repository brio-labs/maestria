# First PR Guide (Maestria)

Welcome to Maestria.
The repository is opinionated: keep the domain pure, keep side effects explicit, keep policies explicit.

## 1) Before you open a PR

1. Read:
   - [docs/PHILOSOPHY.md](./PHILOSOPHY.md)
   - [docs/SPECS.md](./SPECS.md)
   - Your target architecture book in [docs/architecture](./architecture/)
2. Ensure your branch follows repo conventions:
   - `feat/<area>-<short-description>`
   - `fix/<area>-<short-description>`
   - `docs/<short-description>`
   - `chore/<short-description>`
   - `test/<short-description>`
   - `refactor/<short-description>`
3. Keep behavior changes scoped to one subsystem first.
4. Add tests for invariants and behavior changes where relevant.

## 2) PR checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --workspace --all-targets --all-features`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-targets --all-features`
- [ ] `cargo doc --workspace --no-deps --all-features`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features`
- [ ] `cargo deny check all`
- [ ] `cargo machete`
- [ ] `cargo tree --duplicates`
- [ ] `python3 scripts/philosophy-check.py`
- [ ] Required conventional commit format in all commit subjects

## 3) Commit semantics

Use:
```text
<type>(<scope>): <description>
```

Examples:

- `feat(runtime): add deterministic replay hook`
- `fix(domain): harden validation gate transitions`
- `test(domain): add invariant replay case`

This repository enforces format in workflow.

Philosophy checks, format, lints, tests, and dependency hygiene are part of architecture,
not optional ceremony.

Maestria is intended to stay deterministic and composable. The strict gates are non-negotiable.
