# Changelog

All notable Maestria releases are documented here.

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

[0.6.0]: https://github.com/brio-labs/maestria/releases/tag/v0.6.0
