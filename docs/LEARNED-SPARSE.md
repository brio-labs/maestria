# Learned-Sparse Retrieval

## Status

Learned-sparse retrieval is a **research-only**, benchmark-gated candidate lane.
It executes through a non-serving shadow path by default and is not part of the stable product
surface.

The checked-in in-memory provider is a deterministic contract fixture. It proves type,
security, lifecycle, shadow-isolation, and ranking invariants; it is not a trained
learned-sparse model and must not be used as product-quality evidence.

Merging the research foundation does not complete the learned-sparse experiment and must not
transition #90 to a completed state. The issue remains the owner of real-provider integration,
representative judgments, complete measurements, and the final per-query-class
promote-or-retain decision.

## Contract

The canonical representation is `sparse_text_v1`. It is distinct from lexical BM25 and
dense embeddings even when one provider can produce several representation families.

Every sparse identity binds:

- provider, model, revision, and model artifact hash;
- tokenizer and vocabulary hashes;
- vocabulary size and term-ID namespace;
- query and document template hashes;
- preprocessing and weighting versions;
- quantization, pruning threshold, and term cap;
- corpus snapshot and index generation.

Sparse vectors contain a bounded, duplicate-free set of stable term identifiers with finite,
positive weights. Invalid identities, term ranges, weights, representation names, and
generation combinations fail explicitly.

## Generation lifecycle

A retriever cannot be constructed from a raw generation ID. It requires a
`LearnedSparseGenerationCapability` validated against the `IndexGenerationRegistry` entry for
`sparse_text_v1`.

Two capability modes are explicit:

- `Shadow` requires a generation in the registry's `Shadow` lifecycle and can never enter the
  served candidate path;
- `Active` requires the active, serveable generation and can affect served retrieval only for a
  query class authorized by a dated promotion record.

Both modes must match the representation, corpus snapshot, provider/model fingerprint,
vocabulary dimensions, templates, quantization, and preprocessing version. Building,
evaluated, retired, partial, collectable, or incompatible generations cannot execute.

Construction and every query preflight also require the provider and physical index to report
the exact same `SparseIdentity` as the capability. An identity change in either adapter fails
the lane explicitly rather than comparing or serving incompatible rows.

The normal lifecycle remains:

```text
building → evaluated → shadow → active → retired → collectable
```

Activation and rollback remain owned by the shared generation registry. A future physical
sparse backend must not introduce a second lifecycle owner.

## Security and privacy

The lane applies the shared retrieval policy before a candidate score becomes observable:

- scope and ACL;
- trust and sensitivity;
- quarantine and prompt-injection handling;
- current source version;
- secret scanning;
- immutable evidence-snapshot verification.

The current adapter accepts only a local, no-retention provider. A future remote adapter
requires explicit provider disclosure and policy support before query or source content may
leave the instance.

A failed provider, stale identity, privacy rejection, secret-bearing query, incompatible
snapshot, or unavailable generation is an explicit failed/degraded lane. It is never
interpreted as evidence that no relevant source exists.

## Trace and shadow output

Sparse results use the canonical versioned `RetrievalScoreSet`. Each returned candidate has a
`learned_sparse` lane containing the raw fixed-point score, original one-based backend rank,
scale semantics, `sparse_text_v1` representation, and the complete provider/model/tokenizer/
vocabulary fingerprint. Sparse values are never stored as BM25 or dense similarity.

`RetrievalReason::LearnedSparse` contains only the bounded deterministic list of
highest-contributing term IDs and contribution weights. The score and representation identity
have one authoritative location in the lane-score contract rather than a duplicate reason field.

Provider payloads, logits, tensors, hidden states, arbitrary response bodies, and raw shadow
queries are not stored in domain values or shadow observations.

`LearnedSparseShadowStore` records a bounded, serializable runtime observation containing only:

- query ID and classified query class;
- corpus snapshot and generation identity;
- bounded lane status and latency;
- exact evidence/source lineage for bounded candidates;
- the canonical typed learned-sparse lane score;
- bounded learned-sparse contribution provenance.

The store has an explicit capacity, drops the oldest observation when full, and supports JSON
export/import so persistence adapters can store and replay the typed boundary without inventing
a second observation shape.

## Execution policy

`LearnedSparseExecutionPolicy::Shadow` is the default for `RetrievalEngine`.

The engine starts the sparse experiment as a detached, timeout-bounded task and immediately
continues through the existing served lexical/hybrid path. The shadow task has no return channel
into served ranking, evidence, status, coverage, abstention, validation, or completion. Its only
output is a bounded `LearnedSparseShadowObservation` in the independent runtime store.

`Disabled` is an explicit operator opt-out and executes no sparse work. `Active` still requires
a valid, corpus-bound promotion record. A shadow-generation retriever is never eligible for
served fusion, even when another sparse generation has been promoted.

A valid promotion record is produced only by the versioned comparison gate. It is bound to
the corpus, judgment set, evaluation date, model fingerprint, and winning query classes.
Only `SparseFused` can become a served route; `SparseOnly` remains an offline ablation so
neural sparse retrieval cannot replace deterministic exact and lexical foundations.

Removing or invalidating the promotion record restores the existing hybrid route without
reinterpreting stored evidence.

## Evaluation

The compatibility corpus remains:

```text
tests/contracts/learned_sparse_benchmark_v1.json
```

The complete schema-v2 contract fixture is:

```text
tests/contracts/learned_sparse_benchmark_v2.json
```

The real-task freeze is maintained separately from the schema contract fixture:

```text
tests/contracts/learned_sparse_task_corpus_v1.json
```

Its source manifest records repository-relative paths and content hashes. Real Maestria task
cases are primary for normal retrieval, terminology, path, symbol, freshness, and abstention
workloads; synthetic cases are restricted to adversarial and lifecycle failure scenarios. Every
final query class has at least two independent task IDs so no single case can determine
promotion. Final and development cases remain separate, and each case records source inputs,
graded relevance judgments, accepted spans, evidence chains, citation expectations, freshness,
security expectations, and budgets.

Judges use the three-level `NotRelevant`/`Relevant`/`HighlyRelevant` scale independently before
adjudication. Accepted spans must support the cited claim; an evidence chain must reference
known source inputs; abstention is correct only for no-evidence, unsupported, stale, privacy,
provider, or security outcomes. Disagreements are resolved by a third judge, and the frozen
judgment-set hash changes whenever guidance, source inputs, or judgments change.

Schema v2 requires explicit query judgments, a development/final split, source and judgment
hashes, corpus snapshot, index generation, namespace, route budgets, environment, and data
fidelity. Every quality, latency, memory, disk, lifecycle, privacy, security, and energy value
is a typed `Measured`, `Unavailable { reason }`, or `NotApplicable { reason }` measurement.
Missing telemetry is never interpreted as zero.

The comparison requires observations for:

1. lexical baseline;
2. currently eligible hybrid baseline;
3. sparse-only ablation;
4. sparse fused with the existing ranking pipeline.

Promotion records preserve the complete sparse/provider/backend identity, route configuration,
all query-class decisions, final-evaluation and per-class real-task fidelity, declared budgets,
an explicit rollback target, and a content-addressed report hash. Sparse-fused activation is
rejected for protected classes, non-final or synthetic winning classes, mixed identities,
incomplete telemetry, security/privacy failures, and budget violations.

Measurements that exceed a budget remain valid report data. They are recorded with a budget
violation and retain the baseline rather than being rejected or fabricated away.

The committed corpus, deterministic adapters, and shadow-isolation tests are contract evidence
only. A production promotion still requires real Maestria task observations and a dated
benchmark artifact.

When a source document included in the checked-in release benchmark ledger changes, its
source hash and matching snapshot fingerprint must be recomputed. This records input drift;
it does not advance the release stage or convert contract fixtures into real measurements.

## Future provider or backend work

A real provider must implement `LearnedSparseProvider` and pass the shared contract suite.
A real projection must implement `LearnedSparseIndex`, including governed pre-score
filtering, idempotent replacement, deletion/tombstone propagation, rebuild, and deterministic
ordering.

The current durable research projection is the SQLite adapter
`maestria-storage-sqlite::SqliteLearnedSparseIndex`, documented in
`docs/adr/ADR-0006-learned-sparse-projection.md`. It mirrors the shared generation
registry, remains non-searchable outside shadow/active lifecycle states, persists
identity-complete rows, and applies authorization filters before scoring. This
adapter does not make the SQLite choice normative for future providers.

Introducing a new physical backend requires an ADR covering alternatives, affected
invariants, filtering guarantees, migration, recovery, update/delete semantics, operations,
and rollback. Normative architecture must remain provider- and backend-agnostic.
