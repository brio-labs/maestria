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

The table below mirrors the exemption dictionaries in `scripts/philosophy-check.py`
(`MODULE_SIZE_EXEMPTIONS`, `FUNCTION_SIZE_EXEMPTIONS`, `MIXED_RESPONSIBILITY_EXEMPTIONS`,
`ADR_MODULE_EXEMPTIONS`) as of v0.6. Exemptions that no longer match a live
module are pruned from both places; every live exemption has an entry here.

| Path | Owner | Rationale | Expiry |
|------|-------|-----------|--------|
| `crates/kernel/maestria-ports/src/contract_tests.rs` | Kernel team | Shared port contract suite (923 physical lines) keeps every port trait's behavioral conformance in one deterministic fixture family so adapters can run the same suite; split into per-trait contract files (e.g. `graph_contract_tests.rs`, `learned_sparse_contract_tests.rs`, `ocr_contract_tests.rs`) as suites grow. | `v0.7.0` |
| `crates/kernel/maestria-ports/src/in_memory/lexical.rs` | Kernel team | In-memory lexical index module is 480 logical lines (over the 400 module budget); its functions are already within the per-function budget and the module owns one responsibility (in-memory lexical search), so it is exempt pending lexical lane consolidation. | `v0.7.0` |
| `crates/ecosystem/maestria-retrieval/src/visual_benchmark.rs` | Retrieval team | Visual benchmark evidence schema and route evaluation share one versioned benchmark format; the mixed-responsibility signal is accepted while the benchmark format is being stabilized. | `v0.7.0` |

## Consequences

- CI will continue to flag non-conforming modules but will accept an
  exemption-matching annotation.
- Exemptions carry an expiry and a recorded rationale, preventing indefinite
  deferral.
- When an exemption expires, CI will reject the module and force either a
  refactor or a renewal ADR.
- New modules may not claim exemption by default; each exemption requires a
  review and an explicit ADR entry.
