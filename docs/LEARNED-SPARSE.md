# Learned-Sparse Retrieval

## Status

Learned-sparse retrieval is a **research-only**, benchmark-gated candidate lane.
It is disabled from served retrieval by default and is not part of the stable product surface.

The checked-in in-memory provider is a deterministic contract fixture. It proves type,
security, lifecycle, and ranking invariants; it is not a trained learned-sparse model and
must not be used as product-quality evidence.

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
`LearnedSparseGenerationCapability` validated against the active
`IndexGenerationRegistry` entry for `sparse_text_v1`.

The generation must match the representation, corpus snapshot, provider/model fingerprint,
vocabulary dimensions, templates, quantization, and preprocessing version. Building,
evaluated, shadow, retired, partial, or incompatible generations are not serveable.

Construction and every query preflight also require the provider and physical index to
report the exact same `SparseIdentity` as the active capability. An identity change in either
adapter fails the lane explicitly rather than comparing or serving incompatible rows.

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
snapshot, or unavailable active generation is an explicit failed/degraded lane. It is never
interpreted as evidence that no relevant source exists.

## Trace output

Sparse results keep BM25 and dense-similarity fields at zero. They carry a typed
`RetrievalReason::LearnedSparse` containing:

- the fixed-point sparse score;
- `sparse_text_v1` representation identity;
- retrieval-model fingerprint;
- a bounded deterministic list of highest-contributing term IDs and contribution weights.

Provider payloads, logits, tensors, hidden states, and arbitrary response bodies are not
stored in domain values or traces.

## Execution policy

`LearnedSparseExecutionPolicy::Shadow` is the default for `RetrievalEngine`. The presence
of a provider, index, or retriever never activates the lane.

A valid promotion record is produced only by the versioned comparison gate. It is bound to
the corpus, judgment set, evaluation date, model fingerprint, and winning query classes.
Only `SparseFused` can become a served route; `SparseOnly` remains an offline ablation so
neural sparse retrieval cannot replace deterministic exact and lexical foundations.

Removing or invalidating the promotion record restores the existing hybrid route without
reinterpreting stored evidence.

## Evaluation

The frozen contract corpus is:

```text
tests/contracts/learned_sparse_benchmark_v1.json
```

The comparison requires observations for:

1. lexical baseline;
2. currently eligible hybrid baseline;
3. sparse-only ablation;
4. sparse fused with the existing ranking pipeline.

Promotion requires a material quality improvement against **both** lexical and hybrid
baselines, no protected-class regression, complete indexing/update and energy telemetry,
zero privacy/security violations, and no declared budget violation.

Measurements that exceed a budget remain valid report data. They are recorded with a budget
violation and retain the baseline rather than being rejected or fabricated away.

The committed corpus and deterministic adapters are contract evidence only. A production
promotion still requires real Maestria task observations and a dated benchmark artifact.

When a source document included in the checked-in release benchmark ledger changes, its
source hash and matching snapshot fingerprint must be recomputed. This records input drift;
it does not advance the release stage or convert contract fixtures into real measurements.

## Future provider or backend work

A real provider must implement `LearnedSparseProvider` and pass the shared contract suite.
A real projection must implement `LearnedSparseIndex`, including governed pre-score
filtering, idempotent replacement, deletion/tombstone propagation, rebuild, and deterministic
ordering.

Introducing a new physical backend requires an ADR covering alternatives, affected
invariants, filtering guarantees, migration, recovery, update/delete semantics, operations,
and rollback. Normative architecture must remain provider- and backend-agnostic.
