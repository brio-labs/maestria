# Retrieval Score Provenance

## Status

The canonical score-provenance boundary is implemented by `RetrievalScoreSet` and is shared by exact, lexical/BM25, dense, learned-sparse, late-interaction, graph, and named specialized retrieval lanes.

## Invariants

- Every lane records an explicit score kind, raw value, original rank or a typed reason that rank was unavailable, scale semantics, representation identity, and applicable fingerprint components.
- Learned-sparse values are never represented as BM25 or dense similarity.
- Fusion remains rank-based; heterogeneous raw scores are not added or compared without a separately evaluated calibration contract.
- Retrieval reasons contain explanatory metadata only. The authoritative score and representation identity live in the lane-score contract.
- Duplicate score kinds, malformed fingerprints, invalid ranks, and non-canonical score schemas fail closed.

## Persistence migration

SQLite schema v9 canonicalizes legacy candidates that stored fixed `bm25` and `semantic_similarity` fields. Legacy zero fields become absent lanes rather than fabricated measurements. Search trace identity v6 hashes the complete canonical score provenance.

The migration rewrites search-completion payloads, trace identifiers, evidence-pack trace references, and frozen replay keys in one transaction. Reopening an already migrated database is idempotent, and residual legacy score fields are rejected instead of creating parallel compatibility paths.

## Verification

The implementation is covered by domain serialization and malformed-input tests, deterministic trace-identity tests, SQLite migration and replay-reference tests, evidence-pack reproduction tests, learned-sparse retriever and shadow tests, golden-fixture regeneration, and the full repository verification gates.
