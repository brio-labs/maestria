# ADR-0009: Retrieval Audit Event Retention

## Status

Proposed. Decision pending review; implementation follows only after sign-off.

## Context

Every durable search emits retrieval audit events. `SearchKnowledgeCompleted`
carries the full typed plan and outcome inline because rules 41 and 42 require
traces that identify the query, corpus snapshot, index generation, stages,
budgets, filters, and stop reason. Measured on the synthetic harness
(#475-era campaign), one durable search emits roughly thirteen events, and
nothing today bounds or compacts them.

Replay treats the retrieval audit family (`SearchExecuted`,
`SearchKnowledgeCompleted`) as state-free: their appliers validate or no-op.
Their open-time cost is therefore decode and dispatch of every stored row, and
durable-search open cost measurably scales with accumulated events (2.3 s at
~60k events versus 0.62–0.79 s on smaller real corpora, #475/#478). Left
alone, every future session pays a growing tax for searches already
forgotten.

Constraints: rule 40 (important state changes emit append-only domain
events), rule 9 (projections are rebuildable from the log), rule 32 (no
direct database mutation from CLI/API handlers), rule 4 (answer paths remain
auditable). Storage rows satisfy `id == sequence`, so the log has stable
prefix positions.

## Decision

Retention is a governed, append-only retirement marker — never a deletion.

- A new governed input, invoked by an explicit operator maintenance command
  (confirmation-gated like `index --yes`), emits one domain event
  `RetrievalEventsRetired { before_sequence, reason }`. The `reason` is
  mandatory; governance classifies the input as elevated because it narrows
  the audit trail.
- The marker is metadata. Rows remain on disk. The journal loader skips
  decoding retrieval-audit events with `id < before_sequence`; state-relevant
  events in the same range replay unchanged. Observability commands report
  retirement explicitly instead of presenting silent absence.
- Markers only advance: a marker below the existing high-water mark is a
  recorded no-op. Retirement is itself an auditable event, so the audit trail
  ends with a trace of who bounded it and why.
- v1 ships the mechanism and the manual command only. Automatic policy (age
  or size thresholds) is a separate product decision; if adopted later it is
  just another caller of the same governed input.

## Alternatives considered

- **TTL or maintenance daemon deleting rows.** Violates rule 40 (the log must
  stay authoritative and append-only) and rule 32; deletion makes the log
  non-reconstructible and the audit silent.
- **Aggregation into periodic summary events.** Lossy against rule 42: folded
  summaries cannot preserve per-search traces. Summaries themselves
  accumulate, so growth returns at a slower rate with worse forensics.
- **Blob demotion.** Move plan/outcome payloads to the blob store and keep
  digests inline. Bounds bytes but not row count; decode and dispatch cost
  still scales with usage, and trace inspection gains blob reads. Orthogonal
  and possibly complementary later, but insufficient alone.
- **Snapshot plus physical truncation.** The only complete fix for disk
  growth as well, but it requires durable kernel-state snapshots — a new
  persistence mechanism. This ADR's marker is its prerequisite: only retired
  ranges are safe to drop. Deferred, not blocked.

## Consequences

- Open-time decode cost stops scaling with search usage; durable-search open
  returns to corpus-dependent cost.
- Disk growth continues (rows persist) but is bounded by the audit rate
  alone. Physical compaction remains future work that uses the marker as the
  safe-truncation boundary.
- Trace inspection of retired searches returns an explicit "retired at
  sequence N" result; auditability degrades loudly, not silently.
- The domain gains one event variant, which at implementation time requires
  the full codec treatment (payload enum, encode/decode, kind scanner) and
  the release-contract generation tests.
