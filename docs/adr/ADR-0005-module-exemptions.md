# ADR-0005: Module Exemption Policy

## Status

Accepted

## Context

Rules 16–20 of PHILOSOPHY.md mandate that each module owns one named
responsibility at one architectural layer, that `lib.rs` files act as façades
(declaring modules and re-exporting stable APIs), and that implementation bodies
belong in sibling modules. However, several `lib.rs` files and single-module
source files currently contain implementation bodies alongside their re-export
surface. Adapter crates (storage, harness) that implement a single port trait
alongside private helpers are a legitimate pattern, not a boundary violation,
while kernel and runtime façades that accumulate orchestration logic represent
technical debt to be retired.

This ADR establishes an exemption policy with an explicit owner, rationale, and
expiry for every module that does not conform to Rules 16–19.

## Decision

Maintain an `EXEMPTIONS` dict in `philosophy-check.py` that lists each
non-conforming module with:

- **Path** — relative path from the workspace root.
- **Owner** — team member or role responsible for the refactor.
- **Rationale** — why the module is exempt rather than split now.
- **Expiry** — the commit SHA or target milestone after which the exemption
  will be rejected by CI.

The exemption list is checked by `scan_responsibility_maps()` (and any future
check) and exemptions are subtracted from violation counts rather than
suppressing the check output entirely.

### Exempted Modules

| Path | Owner | Rationale | Expiry |
|------|-------|-----------|--------|
| `crates/apps/maestria-daemon/src/lib.rs` | Runtime team | Instance lifecycle, recovery, and runtime construction are tightly coupled lifecycle operations that share adapter wiring state. Splitting into sub-modules risks cyclic construction. Planned as v0.7 decomposition work. | `v0.7.0` |
| `crates/runtime/maestria-runtime/src/lib.rs` | Runtime team | Core runtime struct with 15 sub-modules; the remaining impl bodies serve as the central effect-dispatch entry point. Full decomposition gated on async-effect boundary solidification. | `v0.8.0` |
| `crates/storage/maestria-storage-sqlite/src/lib.rs` | Storage team | Single-adapter crate implementing `EffectJournal` + `EventLog` + repository traits. Helper functions are private to the adapter and do not leak into the public API surface; extracting them would add module-file overhead without reducing conceptual surface. | `v0.7.0` |
| `crates/storage/maestria-search-tantivy/src/lib.rs` | Storage team | Single-adapter crate wrapping Tantivy. Helper modules (`constructors`, `lexical_helpers`, `search_helpers`) exist but the main lib.rs exposes the `TantivyFullTextIndex` struct with impl methods that delegate to those helpers. Acceptable adapter-façade pattern per Rule 19 exception for single-impl adapters. | `v0.7.0` |
| `crates/storage/maestria-graph-sqlite/src/lib.rs` | Storage team | Single-adapter crate with 2 helper modules. The lib.rs exposes `SqliteGraphIndex` struct and its impl; extraction into a sub-module façade would add indirection without benefit. | `v0.7.0` |
| `crates/storage/maestria-vector-sqlite/src/lib.rs` | Storage team | Single-adapter crate with 2 helper modules. Same rationale as graph-sqlite. | `v0.7.0` |
| `crates/harness/maestria-harness/src/lib.rs` | Harness team | Single-adapter crate implementing `HarnessAdapter`. Helper modules (`command`, `process`, `tokenize`) are extracted; the remaining lib.rs implementation belongs to the adapter struct itself. | `v0.7.0` |
| `crates/core/maestria-core/src/lib.rs` | Core team | Core orchestration crate with 9 sub-modules. Remaining impl bodies are service-composition functions that aggregate across modules; decomposition gated on service-layer extraction. | `v0.8.0` |
| `crates/kernel/maestria-governance/src/lib.rs` | Governance team | Governance crate with 9 sub-modules. The remaining `pub const GOVERNANCE_VERSION` is a metadata constant and should not count as an implementation body. Check exemption filters constants. | `v0.7.0` |
| `crates/ecosystem/maestria-retrieval/tests/contract_tests.rs` | Retrieval team | Contract fixture intentionally keeps cross-route invariants together so every route comparison shares one deterministic harness; splitting would obscure the acceptance contract. | `v0.7.0` |
| `crates/apps/maestria-daemon/src/watcher.rs` | Runtime team | Ignore-aware polling, stale transitions, rename detection, and bounded backpressure are one lifecycle state machine; split after watcher pause/resume API stabilization. | `v0.7.0` |
| `crates/apps/maestria-daemon/src/api/services.rs` | Runtime team | Authenticated proposal orchestration currently owns the ordered validation/search/harness/evidence workflow; split after the operation contract stabilizes. | `v0.7.0` |
| `crates/kernel/maestria-domain/src/search_outcome.rs` | Domain team | Search outcome, trace, and provenance value types share serialization and identity invariants; split after the v0.7 trace schema is frozen. | `v0.7.0` |
| `crates/ecosystem/maestria-retrieval/src/repository_benchmark.rs` | Retrieval team | Repository benchmark case, observation, and report types form one published evidence schema; split after the benchmark report format is versioned. | `v0.7.0` |
| `crates/ecosystem/maestria-retrieval/src/lib.rs` | Retrieval team | Public retrieval façade retains compatibility re-exports while route modules migrate; expiry tracks the next route API cutover. | `v0.7.0` |
| `crates/ecosystem/maestria-code-intel/src/lib.rs` | Code-intelligence team | Parser/index adapters share a stable public façade and small helper modules; split after provider provenance is promoted. | `v0.7.0` |
| `crates/ecosystem/maestria-parsers/src/lib.rs` | Ingestion team | Parser registry and format adapters share registration invariants; split after parser capability negotiation is stable. | `v0.7.0` |

## Consequences

- CI will continue to flag non-conforming modules but will accept an
  exemption-matching annotation.
- Exemptions carry an expiry and a recorded rationale, preventing indefinite
  deferral.
- When an exemption expires, CI will reject the module and force either a
  refactor or a renewal ADR.
- New modules may not claim exemption by default; each exemption requires a
  review and an explicit ADR entry.
