# ADR-0006: SQLite Learned-Sparse Projection

## Context

The learned-sparse lane needs a restartable projection for evaluation without making a physical backend part of normative architecture. The provider-neutral `LearnedSparseIndex` port already defines bounded indexing, filtered search, replacement, deletion, clearing, and rebuild. The lifecycle registry remains the owner of generation decisions.

The backend alternatives are:

- weighted postings in the lexical Tantivy adapter, which would conflate lexical and learned-sparse representations;
- a dedicated local projection backend, which adds a second physical dependency and migration surface;
- a remote sparse service, which violates the local-first research default and introduces provider retention and availability concerns;
- SQLite tables in the existing local storage adapter, which keep the projection durable, transactional, restartable, and replaceable behind the port.

## Decision

Use dedicated SQLite tables in `maestria-storage-sqlite` for the research projection. Store one complete serialized sparse identity per generation and one typed persistence row per chunk vector. Keep lifecycle state as a durable mirror of the shared `IndexGenerationRegistry`; require the caller to provide the expected and next typed lifecycle states for every transition.

The projection:

- remains non-searchable while building, evaluated, retired, collectable, or tombstoned;
- binds every row to generation, corpus snapshot, namespace, fingerprint, chunk content hash, and vector identity;
- uses transactional upserts for idempotent indexing and replacement;
- retains tombstone rows for deterministic deletion until rebuild or collection;
- applies the caller's authorization filter before scoring;
- uses deterministic score and chunk-ID ordering with bounded execution accounting;
- treats collection as physical deletion only after a collectable or tombstoned lifecycle;
- retires another active generation in the same namespace during blue/green activation.

## Consequences

- Process restart preserves projection rows and lifecycle state.
- Partial builds cannot become searchable because `Building` is persisted and rejected by search.
- SQLite migration v12 is required for existing stores.
- The physical backend remains an adapter choice; future backends must implement the same port and lifecycle contract without changing domain or retrieval architecture.
- This ADR does not claim provider quality or learned-sparse promotion; the in-memory provider remains a contract fixture until Issue #199 is complete.
