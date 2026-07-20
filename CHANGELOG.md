# Changelog

All notable Maestria releases are documented here.

## [0.6.1] — 2026-07-20

Maestria v0.6.1 adds code intelligence indexing/search, memory promotion,
task completion, and approval resolve commands to the CLI. It also completes
the documentation surface with all supported commands now explicitly listed
in the README and establishes a documentation consistency gate.

### Added

- `index repository` — build and persist Cargo metadata and Rust symbol index.
- `search code` with `symbol`, `path`, `regex`, and `context` subcommands.
- `task complete` — validation-gated completion from a recorded report.
- `memory promote` — governance-gated memory promotion with user approval.
- `approval list` and `approval resolve` — manage pending approval requests.
- Documentation consistency checker (`scripts/doc-consistency-check.py`) that
  derives the command tree from `cli_types.rs` and verifies README coverage.
- Version-to-phase mapping and explicit stable/shadowed/provider-dependent/
  research-only status markers in the roadmap.

### Changed

- README now documents the full CLI surface including all subcommands, daemon
  boundary, code/repository retrieval, task lifecycle, approval management,
  and memory promotion.
- Quick start covers the complete init-to-restart workflow.
- The verify-workspace gate now includes the documentation consistency check.


## [0.6.0] — 2026-07-18

Maestria v0.6 is the **Query-Adaptive Search** release. It turns the retrieval
baseline into a bounded, inspectable search workflow that reports both evidence
and uncertainty instead of treating every query as a fixed top-k lookup.

### Added

- Policy-validated search plans with deterministic intent classification,
  capability checks, scopes, budgets, modalities, freshness, and stop
  conditions.
- Deterministic query rewriting and stage-aware decomposition with rewrite
  accounting in the durable search trace.
- Bounded iterative retrieval with explicit no-evidence, incomplete, conflict,
  stale, warning, and answerable outcomes.
- Evidence packs with claim coverage, source independence, conflict and
  counterevidence metadata, missing evidence, compression lineage, and
  reproducibility fingerprints.
- Governed web discovery/evidence separation: discovery results remain
  candidate URLs until fetched, snapshotted, provenance-checked, and policy
  checked.
- Retrieval and security validators for plan validity, provenance, coverage,
  conflicts, freshness, citation alignment, ACL/trust/quarantine, and
  regression budgets.
- Operator observability commands:
  `search explain`, `search trace`, `search compare`, `index generations`, and
  `evidence coverage`.
- Restart-safe runtime recovery, projection reconciliation, ignore-aware
  directory traversal, task/evidence linking, and source-backed memory
  candidates from the completed earlier milestones.
- Frozen deterministic golden-query fixtures and release-gate execution in CI.

### Changed

- All workspace packages and public component version constants now report
  `0.6.0`.
- The release workflow gates publication on the closed v0.6 milestone, passing
  CI, exhaustive verification, a locked release build, checksum validation, and
  CLI/daemon smoke tests.
- Hyphenated identifier-like queries are treated as exact lookups so common
  filenames, slugs, and secret-like paths do not get misclassified as an
  unsupported natural-language constraint query.

### Verification

The release gate exercises formatting, Clippy policy, workspace tests,
documentation, dependency checks, philosophy checks, the frozen golden gate,
locked release builds, deterministic Linux artifacts, SHA-256 checksums, and
extracted CLI/daemon `--help` smoke tests.

[0.6.1]: https://github.com/brio-labs/maestria/releases/tag/v0.6.1
[0.6.0]: https://github.com/brio-labs/maestria/releases/tag/v0.6.0
