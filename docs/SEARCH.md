# `docs/SEARCH.md`

## Purpose

Maestria search is a typed, budgeted, auditable retrieval capability. It is not a fixed sequence of vector lookup, top-*k* selection, and prompting.

Search produces evidence candidates and coverage information for downstream validation, reasoning, and decisions. It does **not** establish external factual truth. Maestria can preserve source-backed observations and enforce internal state invariants; it cannot make an external claim true.

This document defines the search architecture. General system invariants and design principles are defined in:

- [`docs/SPECS.md`](SPECS.md)
- [`docs/PHILOSOPHY.md`](PHILOSOPHY.md)

## Scope

This document covers:

- typed search plans and outcomes;
- corpus and index identity;
- candidate retrieval, fusion, reranking, and expansion;
- evidence provenance and coverage;
- budgets, stopping, and abstention;
- search traces and reproducibility;
- adapter and runtime boundaries;
- retrieval evaluation and replacement policy.

It does not define a permanent model, database, vector index, ranking algorithm, or external provider.

## Normative Principles

1. Every non-trivial search executes from a validated `SearchPlan`.
2. A model may propose a plan; runtime and policy validate and constrain it.
3. Search is bounded by explicit resource, scope, freshness, and evidence requirements.
4. Every candidate maps to a source artifact version and precise evidence span where applicable.
5. Search results are observations and evidence references, not guaranteed truth.
6. Evidence snapshots are immutable. Trust, freshness, conflict, and validity annotations are versioned.
7. Provider-specific payloads remain inside adapters.
8. Search implementations are replaceable until benchmark results justify a choice.
9. No model name, public leaderboard, or architecture diagram proves that retrieval works for Maestria.
10. Material retrieval changes require evaluation against a versioned Maestria query set.

## Domain State and External Truth

Search participates in an epistemic pipeline:

```text
Source
  → observation
  → evidence snapshot
  → uncertain claim
  → governed memory candidate
  → policy-based decision
```

These objects have different meanings:

| Object | Meaning |
|---|---|
| Source | An external or local origin of information |
| Evidence | An immutable, source-backed representation and location |
| Claim | A normalized proposition that may be stale, disputed, or unsupported |
| Memory | A promoted claim retained under policy |
| Decision | An action selected using evidence and governance |
| Validation | A check that required support is present and acceptable |

Search can retrieve evidence for a claim. It cannot prove that the source is correct, current, or truthful. Those properties require freshness checks, corroboration, live validation, or human review as appropriate.

Trust labels, freshness statuses, and conflict annotations are governance metadata. They must not be represented as guarantees of external truth.

## Search Boundary Objects

Search plans, candidates, outcomes, and traces are typed boundary objects.

### Search Plan

```rust
pub struct SearchPlan {
    pub query_id: QueryId,
    pub original_query: String,
    pub intent: SearchIntent,
    pub scope: CorpusScope,
    pub snapshot: CorpusSnapshotId,
    pub freshness: FreshnessRequirement,
    pub modalities: ModalitySet,
    pub stages: Vec<SearchStage>,
    pub budget: SearchBudget,
    pub stop: StopConditions,
    pub evidence: EvidenceRequirements,
}
```

A plan must specify:

```text
query intent
corpus and trust scope
ACL and sensitivity scope
corpus snapshot
required freshness
modalities
candidate retrieval stages
fusion and reranking policy
context expansion policy
quality and resource budgets
stop conditions
required evidence coverage
```

A plan is a proposal until it passes:

```text
schema validation
capability validation
scope and ACL checks
governance checks
budget checks
snapshot and freshness checks
representation compatibility checks
```

### Search Intent

The intent taxonomy is extensible:

```rust
pub enum SearchIntent {
    ExactLookup,
    FactualLocal,
    SemanticDiscovery,
    CompositionalConstraints,
    MultiHop,
    CorpusSynthesis,
    RepositoryCode,
    VisualDocument,
    TemporalMemory,
    CurrentWeb,
    ContradictionAudit,
}
```

Intent affects routing and evidence requirements. It does not force a particular implementation.

### Evidence Candidate

```rust
pub struct EvidenceCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub scores: RetrievalScoreSet,
    pub trust: TrustLabel,
    pub freshness: FreshnessStatus,
    pub duplicate_cluster: Option<DuplicateClusterId>,
    pub reasons: Vec<RetrievalReason>,
}
```

A candidate must preserve:

- source artifact and version;
- exact text, page, line, symbol, table, or visual region when available;
- retrieval and ranking reasons;
- snapshot and representation identity;
- duplicate and conflict relationships;
- applicable trust, ACL, and freshness annotations.

Generated summaries and contextual retrieval text may improve ranking but never replace the raw source evidence.

### Search Outcome

```rust
pub struct SearchOutcome {
    pub evidence: Vec<EvidenceCandidate>,
    pub coverage: EvidenceCoverage,
    pub conflicts: Vec<ConflictSet>,
    pub trace: SearchTraceId,
    pub status: SearchStatus,
}
```

Possible statuses include:

```text
answerable
answerable_with_warnings
evidence_incomplete
sources_conflict
stale_evidence_only
no_evidence_found
denied_by_policy
quarantined_for_review
abstained
```

An outcome may be useful without being complete. Missing coverage and unresolved conflicts must be explicit.

## Search Execution Model

The logical execution flow is:

```text
query request
  → plan proposal
  → plan validation
  → scope/snapshot selection
  → parallel candidate generation
  → identity normalization and deduplication
  → fusion
  → reranking
  → constraint and coverage checks
  → context or graph expansion
  → evidence packing
  → stop, continue, live-verify, or abstain
```

This is a capability graph, not a mandatory fixed pipeline. Stages may be skipped, repeated, or reordered when the plan and policy permit it.

### Candidate Retrieval Lanes

Implementations may provide any compatible combination of:

| Lane | Examples of capability |
|---|---|
| Deterministic | exact IDs, paths, symbols, metadata, phrase or pattern lookup |
| Lexical | fielded text retrieval, aliases, acronyms, language-aware matching |
| Neural | dense, sparse, multi-vector, or multimodal retrieval |
| Structural | parent/child hierarchy, repository graph, citation graph, temporal graph |
| External | approved filesystem checks, harness operations, web discovery, provider APIs |

A retriever is selected by capability, scope, budget, and measured quality. No lane is universally required.

### Fusion and Ranking

Fusion and ranking must account for the query and evidence requirements, not only similarity.

Supported strategies may include:

```text
rank-based fusion
calibrated score fusion
weighted fusion learned from a judged set
cross-encoder or equivalent reranking
late-interaction reranking
constraint or listwise verification
```

Raw scores from unrelated retrievers must not be combined without calibration or a validated fusion policy.

Ranking should support:

- relevance to the complete query;
- individual constraint satisfaction;
- source and section diversity;
- freshness;
- duplicate suppression;
- trust-zone and ACL filtering;
- conflict and counterevidence coverage;
- evidence utility and citation precision.

### Context Expansion

Expansion is performed after candidate selection where possible. It may include:

```text
parent sections
neighboring paragraphs
definitions and references
code callers, callees, implementations, or tests
full table headers and rows
figure captions and regions
citation context
bounded graph neighbors
```

Expansion must remain bounded by the plan and preserve lineage to the original source span.

## Budgets and Stop Conditions

`SearchBudget` should be able to constrain:

```text
maximum stages
maximum candidate count
maximum reranking count
maximum iterations
maximum latency
maximum model or token use
maximum bytes read or fetched
maximum web queries/pages
maximum concurrency
maximum live verification operations
```

A search may stop when:

```text
required evidence coverage is met;
freshness and source-diversity requirements are met;
marginal evidence gain is below the configured threshold;
the resource budget is exhausted;
the evidence is conflicting and requires review;
no supported evidence is available;
policy forbids further retrieval.
```

Exhausting a budget is not equivalent to finding no evidence. The stop reason must be recorded.

Agentic or iterative retrieval additionally requires:

```text
cancellation
bounded query generation
bounded corpus and domain scope
bounded external operations
complete trace output
explicit abstention behavior
```

## Identity, Snapshots, and Compatibility

Search must use strong identifiers rather than untyped strings:

```text
QueryId
SearchTraceId
CorpusSnapshotId
IndexGenerationId
ArtifactVersionId
EvidenceId
DuplicateClusterId
ConflictSetId
RetrievalModelFingerprint
```

A corpus snapshot identifies the source versions visible to a search. An index generation identifies a particular materialized retrieval projection.

Every persisted representation must identify its compatibility metadata, including as applicable:

```text
provider or implementation identity
model or algorithm revision
artifact or model hash
dimensions and quantization
preprocessing and query/document templates
representation schema version
```

Representations with incompatible fingerprints must not be compared as if they shared the same scoring semantics.

Index or representation changes use generations:

```text
building
  → evaluated
  → shadow
  → active
  → retired
  → collectable
```

Activation is atomic and rollback must retain the previous compatible generation for a defined window. Old representations are never silently reinterpreted.

## Provenance, Deduplication, and Conflict

### Provenance

Every candidate and evidence pack must answer:

```text
What source produced this?
Which artifact version was used?
What exact span or region supports it?
Which snapshot and index generation were active?
Which retrieval stages selected it?
Which transformations produced the displayed excerpt?
```

### Deduplication

Deduplication may use:

```text
exact content identity
normalized text identity
near-duplicate fingerprints
canonical URL or source clustering
generated-file rules
repository and version identity
```

Deduplication improves efficiency and diversity. It must not erase the source lineage of retained evidence.

### Conflict

For high-risk, stale, disputed, or decision-relevant searches:

```text
retrieve possible counterevidence;
identify newer or superseding versions;
group contradictory evidence;
classify the conflict;
resolve under policy, report uncertainty, or abstain.
```

Contradictory candidates must not be silently collapsed into a single apparently certain result.

## Evidence Coverage and Packs

Search should return a structured evidence pack rather than an undifferentiated result list.

An evidence pack should contain:

```text
query and plan identity
corpus snapshot and index generation
required claims or subquestions
claim-to-evidence coverage
source, version, freshness, and trust metadata
retrieval reasons and score trace
duplicate and conflict information
missing evidence
compression lineage
stop or abstention reason
```

Coverage is a structured status, for example:

```text
supported
partially_supported
unsupported
contradicted
not_checked
```

Retrieved content is data. It must remain in a clearly delimited data channel and must not modify policy, tool authorization, system instructions, or approval requirements.

## Search Trace

A `SearchTrace` is an audit and reproducibility artifact, not authoritative domain state.

It records:

```text
original query and rewrites
intent and selected route
scope, ACL, trust zone, and freshness requirements
corpus and index snapshots
representation fingerprints
retrievers and budgets
candidate ranks and scores
fusion and reranking decisions
filters and exclusions
duplicate clusters
context expansions
missing evidence slots
conflicts and counterevidence
cancellation, timeout, or failure events
stop or abstention reason
```

A stored result should be reproducible from its trace, source snapshots, index/model generations, and policy profile. Live sources that cannot be frozen must be marked non-reproducible and revalidated before reuse.

## External and Live Retrieval

Web access, harness execution, live filesystem checks, and other side-effecting or trust-boundary operations are separate governed capabilities.

Discovery is not evidence:

```text
search result or snippet
  → candidate reference

fetched and preserved source
  → evidence subject to provenance, policy, freshness, and validation
```

Live operations must specify:

```text
allowed scope
approval requirements
network or filesystem policy
maximum operations
freshness requirement
snapshot or artifact retention
validation method
```

A search result or external provider response does not become domain truth merely because Maestria stored it.

## Runtime and Crate Boundaries

`maestria-retrieval` owns:

```text
SearchPlan
retrieval DAG
candidate routing
fusion and reranking
expansion
coverage and stopping
evidence pack construction
SearchTrace generation
```

`maestria-domain` owns domain-shaped state and transitions. It may emit a task-significant effect such as:

```rust
MaestriaEffect::SearchKnowledge(SearchRequest)
```

`maestria-runtime` executes effects, invokes adapters, and maps outputs back into `DomainInput`. It must not mutate domain state directly.

Provider-specific query, response, index, model, and storage types remain in adapters. App crates may compose services but must not add policy shortcuts or direct domain mutations.

## Replacement and Benchmark Policy

All retrieval implementations are replaceable until benchmarked.

This includes:

```text
retrievers
ranking and fusion methods
embedding or representation models
parsers and chunking strategies
vector or lexical backends
graph implementations
rerankers
compression methods
hardware and deployment profiles
```

A replacement is eligible for activation only after evaluation against the relevant Maestria workload. Evaluation must include, as applicable:

```text
Recall@k
nDCG, MRR, or equivalent ranking quality
exact-span recall
claim and evidence-chain coverage
citation precision
source diversity and redundancy
conflict detection
abstention quality
ACL leakage and poisoning resistance
p50/p95/p99 latency
RAM, disk, indexing, update, and energy cost
```

Public benchmarks and vendor claims may inform experiments but are not acceptance evidence for Maestria.

## Retrieval Evaluation Gate

Maintain a versioned evaluation set derived from real tasks. Each judgment records:

```text
query and intent class
corpus snapshot
relevant artifacts and exact spans
required evidence chain
freshness requirement
trust and sensitivity constraints
correct abstention behavior
```

Evaluation runs must record:

```text
implementation and model fingerprints
index generation
corpus snapshot
configuration and budgets
hardware/deployment profile
quality and resource results
regressions and known limitations
```

CI policy:

```text
Pull request:
  small golden set; no material quality or security regression

Main branch:
  full retrieval, robustness, compatibility, and migration suite

Release:
  frozen benchmark report with all fingerprints and corpus identities
```

No retrieval change is complete until its quality, cost, security, and reproducibility impact is known.