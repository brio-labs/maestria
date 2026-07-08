# Maestria Philosophy

This document is enforceable. CI and review block violations.

## Rules

1. Domain code must be deterministic: no time, network, filesystem, process calls, random sampling, or hidden global state.
2. All I/O-capable work must be represented as explicit effect values and executed by runtime/adapters after governance review.
3. Evidence must be typed and source-grounded; raw strings are not evidence when the provenance shape is known.
4. Every factual answer path should be auditable through event, command, validation, or evidence trail.
5. The repo must maintain a conservative, local-first baseline; remote services are adapters.
6. Policy and mechanism stay separate. Governance may classify and decide; it must not execute effects.
7. Task completion is validation-gated. Generic unvalidated completion states are not allowed in the domain.
8. Memory candidates and promoted memories must point back to evidence. LLM output can propose; it cannot silently promote.
9. Projections are rebuildable. Search, vector, and graph stores never own truth.
10. Kernel crates cannot depend on adapter/runtime/provider crates such as Tokio, SQLx, reqwest, Tantivy, or Axum.
11. Domain production code must not contain `unwrap`, `expect`, or `panic` paths.
12. Bare task markers are not allowed in source, config, or docs.

## Enforcement

- `scripts/philosophy-check.py`
- Workspace lint, documentation, and test gates in CI
- Contract checks for kernel inputs/outputs and transitions
- Review through CODEOWNERS on invariant-owning surfaces
