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
   cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::too_many_lines -D clippy::cognitive_complexity -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
   cargo test --workspace --all-targets --all-features
   cargo test --workspace --doc --all-features
   cargo doc --workspace --no-deps --all-features
   RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
   cargo deny check all
   cargo machete
   cargo tree --duplicates
   python3 scripts/philosophy-check.py
   python3 -m unittest discover -s scripts -p 'test_*.py'
   ```
5. Update docs (`README.md`, `docs/PHILOSOPHY.md`, or `docs/SPECS.md`) when behavior or invariants
   change.

## Quality standards

- **Mechanism / policy separation:** domain transitions are deterministic and side-effect free.
- **No implicit I/O in core logic:** file / process / network work belongs in adapters.
- **Explicitness over magic:** gate decisions should be visible in the API and tests.
- **Small and boring:** prefer straightforward types and explicit data over abstraction layers.
- **Module boundaries:** each module owns one named responsibility at one architectural layer. Split when a second independently testable concept, representation, lifecycle, or contract appears; use typed APIs, traits, and effects across boundaries instead of appending unrelated code. Public façades expose stable boundaries and re-export implementations.
- Production Rust functions stay below 100 logical lines; any exception names a time-bounded ADR.
- Rust source files stay below 900 physical lines, including tests. Split responsibility-specific modules instead of adding exemptions.
- Every concrete port adapter runs shared contract tests and adapter-specific boundary tests.
- Runtime lifecycle policy has one owner; application entry points do not duplicate recovery or shutdown orchestration.

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
