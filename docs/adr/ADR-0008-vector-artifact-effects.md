# ADR-0008: Per-Artifact Vector Effects

## Status

Proposed. Implementation follows only after review sign-off.

## Context

Dense (vector) ingest currently emits one `IndexVector` effect per chunk.
A 250-file slice produces ~3,570 effects, each of which:

- opens the embedding provider transport for a single chunk (sequential
  round-trip, ~25 ms compute-bound on CPU per call),
- serializes its own SQLite upsert of the chunk's vectors,
- contends on the vector lane semaphore (`VECTOR_LANE_PERMITS = 8`,
  #476) with every other chunk of the same artifact.

Measured dense-ingest cost for the slice: 81.8 s, of which the embedding
sidecar alone accounts for a large constant floor. The provider-side
batching groundwork (`EmbeddingProvider::embed_batch`, #477) exists but
has no caller: batching is only meaningful when a single effect sees many
chunks at once.

### Measured negative result: naive windowing

A branch-level experiment (#477 era) restructured per-chunk effects into
windows: the first `IndexVector` effect for an artifact took a per-artifact
lock, collected all of the artifact's pending chunks, embedded them in one
`embed_batch` call, and upserted in one transaction; sibling per-chunk
effects found their chunks already embedded and no-opped. The result was a
3.1× regression: 254 s vs the 81.8 s baseline. The lock did not merely
serialize embeds — it serialized against the session-end drain and against
sibling effects across artifacts holding the same lane permits while
waiting on the lock, converting the lane's useful concurrency into a
convoy. The experiment was reverted; the design problem is real but the
mechanism was wrong.

### Why per-chunk effects cannot be batched safely today

Batching at the effect boundary requires the effect to observe chunks
whose `chunk_registered` events have already been applied to
`KernelState`. Today an artifact's chunks register as separate events, and
a per-chunk effect fires per chunk with no knowledge of its siblings —
the runtime has no notion of "artifact X still has pending chunks", so
any effect-side aggregation needs bookkeeping outside the domain (the
lock/set experiment) or a changed effect contract.

## Decision

Replace per-chunk `IndexVector` effects with one per-artifact effect,
`IndexArtifactVectors`, mirroring the full-text lane's shape
(`handle_index_full_text` operates per artifact with `FullTextLocks`).

- **Effect contract.** `IndexArtifactVectors { artifact_id }` carries the
  artifact identity only; the effect handler reads the artifact's current
  chunk set from `KernelState` at execution time (borrowed, rule 58) and
  embeds all its chunks in one `embed_batch` call, then performs one
  transactional upsert of the artifact's vector rows.
- **Pending-vector tracking in the domain.** `KernelState` tracks, per
  artifact, the count of registered chunks whose vectors are not yet
  indexed (`pending_vector_chunks`). The domain emits
  `IndexArtifactVectors` when an artifact's parse completes (transition to
  `Parsed`/`Indexed` boundary), not per chunk. Whether vectors are already
  served remains observable state: a chunk is vector-indexed or not, and
  the artifact-level effect is emitted exactly once per artifact
  generation. Incremental re-parses (parser recovery, content change)
  re-emit the effect for the new generation.
- **Total effect.** The runtime handler is total for the artifact: it
  embeds every chunk of the current chunk set, so no sibling effect or
  lock is needed (rule 23). A failure degrades the artifact's vector
  lane explicitly (existing degraded-artifact path), never silently.
- **Lane and permits unchanged.** One `IndexArtifactVectors` effect
  occupies one vector-lane permit, so 8 artifacts embed concurrently; the
  sidecar batch curve (8–32 batch width, ~16 ms/chunk compute floor)
  applies per artifact window instead of per chunk.
- **Persistence.** Vector rows keep the #474 representation contract
  (kinds + `representations_digest`); the single-transaction upsert per
  artifact replaces per-chunk transactions. The durable event stream gains
  no new event kind: vector indexing remains a projection-side effect of
  the artifact's parsed state; `pending_vector_chunks` is derivable state
  within `KernelState` from existing chunk/index events, replayed by
  `apply_event` (rule 9, rule 40).

## Alternatives considered

- **Per-artifact locks around per-chunk effects** (the measured negative
  result). Rejected with data: 254 s vs 81.8 s. Locks cannot create
  batching without a domain-level notion of artifact completion.
- **Keep per-chunk effects, batch opportunistically in the provider
  adapter.** Rejected: transport-level batching across unrelated effects
  reintroduces hidden coordination (queues, timers) inside an adapter,
  against rules 2 and 28, and still pays one effect per chunk.
- **Status quo.** Rejected: leaves the measured 81.8 s dense lane with a
  per-chunk transport floor that batching removes; projected
  ~40–50 s for the 250-file slice once `embed_batch` is consumed.

## Consequences

- Dense ingest no longer scales effect count with chunk count; the event
  log and effect dispatch shrink by the chunks-per-artifact factor
  (~14× on the harness slice).
- The `IndexVector` effect variant is removed from the domain contract
  (rule 23: every emitted effect has one execution; a variant no runtime
  executes must be deleted). All constructors, dispatch, tests, and any
  persisted effect payloads migrate in the same change (rule 29).
- `pending_vector_chunks` is new `KernelState` state: replay, snapshot,
  and golden-gate fixtures must account for it.
- The embedding sidecar's batch floor (~16 ms/chunk CPU) becomes the
  dense-lane lower bound per artifact; further wins require the sidecar
  (GPU or smaller quantized model), not the runtime.
- Verification plan: fresh-instance dense slice benchmark vs the 81.8 s
  baseline (method: `docs/BENCHMARKING.md`), golden-gate green, and the
  provider unittest suite extended for multi-chunk `embed_batch`
  position/ordering cases.
