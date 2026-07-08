# Contributing to Maestria

> **⚠️ STOP.** Before writing code, read [`docs/PHILOSOPHY.md`](docs/PHILOSOPHY.md).

Maestria inherits its enforcement posture from Brioche: behavior gates are part of the
implementation flow, not a post-check.

## Prerequisites

- Rust (stable toolchain, 1.95+ recommended)
- `rustup component add rustfmt clippy`
- (Optional but recommended) `cargo install cargo-deny cargo-machete`

## Repository setup

```bash
rustup toolchain install stable
rustup component add rustfmt clippy
```

## Development workflow

1. Read [`docs/first-pr-guide.md`](./docs/first-pr-guide.md).
2. **Create a branch** from `main`:
   - `feat/<area>-<short-description>`
   - `fix/<area>-<short-description>`
   - `docs/<short-description>`
   - `chore/<short-description>`
   - `test/<short-description>`
   - `refactor/<short-description>`
3. Make changes in one logical layer first (`maestria-domain` or `maestria-governance`).
4. Run quality gates locally (minimum):
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace --all-targets --all-features
   cargo test --workspace --doc --all-features
   cargo doc --workspace --no-deps --all-features
   RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
   cargo deny check all
   cargo machete
   cargo tree --duplicates
   python3 scripts/philosophy-check.py
   ```
5. Update docs (`README.md`, `docs/PHILOSOPHY.md`, or `docs/SPECS.md`) when behavior or invariants
   change.

## Quality standards

- **Mechanism / policy separation:** domain transitions are deterministic and side-effect free.
- **No implicit I/O in core logic:** file / process / network work belongs in adapters.
- **Explicitness over magic:** gate decisions should be visible in the API and tests.
- **Small and boring:** prefer straightforward types and explicit data over abstraction layers.

## Commit message format

Use:

```text
<type>(<scope>): <description>
```

Where:

- `<type>` is one of:
  - `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `invariant`
- `<scope>` is a short subsystem id (`domain`, `governance`, `runtime`, `tests`, `ci`, `repo`, etc.).
- `<description>` is imperative and concise (max 100 chars).

Examples:

```text
feat(governance): add approval policy deny path for critical operations
fix(domain): tighten validation gate status checks
chore(ci): add dependency audit to CI
```

## Checklist before opening a PR

- [ ] Code is formatted and lint-clean (`fmt`, `clippy`).
- [ ] Tests pass (`cargo test`).
- [ ] Dependency hygiene pass (`cargo deny check all`, `cargo machete`, `cargo tree --duplicates`).
- [ ] Documentation checks pass (`docs`, `specs` updates if required).
- [ ] Philosophical guardrails pass (`python3 scripts/philosophy-check.py`).
- [ ] Commit message uses the required format above.
