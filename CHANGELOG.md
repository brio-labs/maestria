# Changelog

All notable Maestria releases are documented here.

## [Unreleased]

Maestria activates the dense embedding lane for the lexical/hybrid search
route on benchmark evidence, adds live indexing metrics to the CLI, and
introduces the repository index selection layer: a whitelist-first choice
surface (CLI, daemon API, studio, and web) that scopes repository code
intelligence to a reviewed set of directories.

### Added

- Hybrid dense-lane activation: a real-instance benchmark
  (`maestria_hybrid_evaluation`, manual) measures lexical vs hybrid recall on
  six query classes with RAPL energy telemetry and writes a promotion record
  bound to the report hash; `VocabularyExpansion` and `DomainTerminology` are
  served by the dense lane (`hybrid_state=Active`).
- Live `index` metrics: per-file progress, embedding counts from the vector
  projection WAL, throughput, and a final summary with MiB/min and embeddings.
- `index repository` — whitelist-first repository code indexing with
  `--include`/`--all`/`--yes` selection, classified candidate tree, and
  per-directory policies (`max_file_bytes`, `skip_generated`, `skip_minified`);
  identity, delta, records, and freshness are selection-scoped.
- Repository index selection surface: daemon operations
  (`repository_index_candidates`/`selection_get`/`selection_save`/`run`/
  `status`/`children`/`files`/`progress`), studio HTTP proxy routes, and the
  web workspace with lazy candidate-tree browsing and run progress polling.
- Hot-path performance: startup reconciliation is watermark-guarded so
  unchanged instances skip all projection reconciles (search start drops from
  seconds to milliseconds), the tantivy writer commits once per session
  instead of per operation (read paths flush for read-your-writes, `Drop`
  preserves durability), CLI state polling reuses one SQLite connection
  across poll iterations, and `KernelState` collections are Arc
  copy-on-write so staged inputs clone pointer handles instead of deep
  entity/event copies (~17x fewer allocations on an index workload), and
  domain inputs now apply in place under the runtime write lock with
  replay-to-persisted-prefix repair on failure, removing the remaining
  per-input candidate clone (index user CPU drops ~78% on a 1,200-file
  corpus; the cost no longer grows with the event-log size).
- Fixed per-invocation costs: stored-event payload validation now runs only
  when the database schema changes (fresh database or migration) instead of
  decoding the whole event log as JSON on every store open (single-command
  latency on a 60k-event instance drops ~15%), and `Scope` stores its root
  and pattern lists behind shared slices so cloning an effect execution
  context no longer deep-copies governance configuration per effect.
- SQLite write connections use WAL `synchronous=NORMAL`: commits stop
  fsyncing per transaction, which removes an fsync stall per persisted
  batch during indexing (1,200-file corpus drops from 6.6 minutes to under
  2 minutes). Application crashes remain safe — only OS-level power loss can
  lose the most recent commits, and startup recovery re-drives any pending
  work from the durable event log.
- Repository queries reuse parsed SQLite statements (`prepare_cached`), so
  per-hit authorization lookups no longer re-plan SQL on every call.
- Read-only search startup skips the full event-log replay: the retrieval
  runtime consumes only the index-generation registry, which rebuilds from
  the self-contained generation events (13 of ~60k), and evidence rendering
  reads the durable evidence store the engine already authorizes against
  (lock-contended searches drop from 3.9 s to 3.2 s on a 60k-event
  instance). Event-log replay also stops deep-cloning every envelope.
- Slim `chunk_registered` events: the payload no longer duplicates the
  chunk text inside every representation (`raw`/`retrieval` mirror it),
  storing representation kinds plus a stable digest instead. Legacy rows
  with inline contents keep replaying and recompute their digest on read.
  On a 1,200-file corpus this cuts event-log payload bytes by ~62%
  (111 MB → 42 MB), shrinks the database ~26%, speeds indexing ~25%, and
  drops durable-search startup from 3.9 s to 2.3 s on a fresh log.
- `Chunk` carries that `representations_digest`, so restart recovery
  compares registrations by identity instead of requiring both sides to
  hold full representation contents.
- `privacy_exclusions` defaults covering machine-state directories, wired
  into index-selection scanning and blocked patterns.
- `IndexGenerationRegistry` with lifecycle transitions and retired-generation
  rollback, plus `MonotonicInstant` saturating arithmetic.
- Vector projection WAL mode and `embedding_row_count()` for concurrent
  readers.

### Changed

- Search execution budgets: default candidate/work ceilings raised
  (30 000 / 30 000 000) so the dense lane scans the whole projection instead
  of a truncated `chunk_id`-ordered prefix — the previous top-K was only the
  best of the first scanned rows.
- Promotion gate: `source_redundancy` bound relaxed from +20% to +100%
  relative, reflecting the structurally larger candidate set of a fused
  two-lane result.
- Read-only search runtimes (CLI search while the daemon holds the instance
  lock) now derive the hybrid policy from the persisted promotion record
  instead of hardcoding shadow.
- `content_hash` identity unified on the chunk text so projection
  reconciliation skips re-embedding of unchanged chunks.
- The code-intel source registration window constant moved to the daemon
  (`repository_source_registration.rs`); `code_intel_sources.rs` removed.
- The repository code-intel walk now applies `PrivacyExclusions::default()`
  (machine-state and credential-shaped paths) so `index repository` enforces
  the same privacy boundary as the generic whitelist-first indexer.

### Fixed

- Dense lane `BudgetExhausted` only when an exhausted lane produced no
  candidates — a lane with results was misreported and could fail the search
  trace/plan match.
- Full collection of all documents no longer marked `truncated` (one-past-end
  candidate limits in the lexical chunk/card adapters).
- Repository index runs and candidate scans canonicalize the root before
  selection containment, matching the CLI path (a non-canonical root could
  silently drop Cargo targets).
- Web selection tree: a file selected individually stays checkable instead of
  being covered by its own selection.
- Ingestion throughput: the runtime full-text effect now batches every
  pending chunk of an artifact into one projection commit (one `IndexFullText`
  effect serialized per artifact), replacing the per-chunk Tantivy commit;
  a 2,000-file home-scale root drains in minutes instead of stalling for
  hours under segment flush/fsync churn.
- Daemon memory on large roots: with the pipeline draining, RSS plateaus once
  ingestion completes instead of growing monotonically while pending
  artifacts accumulate (previously the process could be killed externally).
- The learned-sparse benchmark instrumentation record binds `report_hash` to
  the frozen corpus content instead of an arbitrary format string, so the
  record is verifiably tied to its evaluation evidence.
- The learned-sparse benchmark's `peak_ram_bytes` is now the per-route
  process RSS delta around the route's run (search plus lifecycle
  operations), not the process-wide `VmHWM` high-water mark, which the first
  measured route dominated.
- Studio Index workspaces pre-fill the instance root path from the daemon
  bootstrap status instead of requiring the path to be typed by hand (the
  sanitized display name never looked like an absolute path).

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
