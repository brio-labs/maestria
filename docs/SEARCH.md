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
11. Original query identity is immutable; rewrites are additional typed retrieval views.
12. Deterministic expansions precede model proposals, and stage roles restrict where each rewrite may execute.

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
    pub index_generation: IndexGenerationId,
    pub freshness: FreshnessRequirement,
    pub modalities: ModalitySet,
    pub stages: Vec<SearchStage>,
    pub budget: SearchBudget,
    pub stop: StopConditions,
    pub evidence: EvidenceRequirements,
    pub authorization: RetrievalPolicySnapshot,
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
authorization snapshot
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

### Retrieval score provenance

`RetrievalScoreSet` is a versioned collection of homogeneous lane scores. Every lane records an
explicit kind (`exact`, lexical/BM25, dense similarity, learned sparse, late interaction, graph,
or a named specialized route), its raw score, original backend rank or a typed reason that rank
was unavailable, scale/normalization semantics, representation identity, and complete applicable
fingerprint components. Lanes are canonicalized deterministically and duplicate score kinds fail
closed.

Legacy persisted candidates containing only `bm25` and `semantic_similarity` migrate once through
the SQLite schema-v9 migration. Legacy zero fields become absent lanes rather than fabricated zero
measurements. Search trace identity v6 hashes the complete score provenance, and evidence-pack
trace and frozen replay references are rewritten transactionally. No legacy/new parallel score
path remains after migration.

Rank fusion remains rank-based. Raw values from heterogeneous lanes are never added or compared
without a separately evaluated calibration contract.

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

When a specialized local intent is unavailable, planning uses one bounded governed
local-text fallback instead of retrying or silently changing scope. The trace keeps
the classified intent and a typed route decision, and records a degradation such as
`governed local-text fallback for unavailable TemporalMemory intent`; malformed
plans still return typed errors. The fallback is intentionally non-promotional:
specialized routes remain disabled until their frozen benchmark proves a quality and
resource win.

### Search Trace and Golden Gate

Every runtime-produced `SearchOutcome` carries a typed trace payload in addition
to its stable `SearchTraceId`. The payload records the original query, plan
identity, corpus and index fingerprints, retriever route, budgets, candidate
ranks and scores, pre-scoring security filters, fusion policy, typed rewrite
origins and stage roles, expansion summary, missing evidence, conflicts, and
deterministic stop reason.
When a runtime policy is available, its deterministic policy fingerprint is
included; expansion counts are nullable when an adapter cannot expose the
pre-expansion delta, rather than claiming a fabricated count.
`SearchKnowledgeCompleted` event persists this payload with the outcome; older
event payloads remain readable without trace details.
`SearchTraceId` uses a versioned deterministic identity seed. New traces use
identity version 2; traces serialized before this field existed default to the
legacy identity algorithm so their historical IDs remain verifiable.


The versioned golden gate evaluates frozen judgments using deterministic
Recall@k, nDCG@k, MRR, and exact-span recall metrics alongside latency,
memory/disk, ACL-leakage, and prompt-injection/poisoning attack-success
measurements. Configured regressions return an error and therefore fail the
test/CI gate rather than being printed as advisory metrics.

## Query Rewriting and Stage Roles

The original query is preserved in every rewrite session and trace. Rewrites
are additional retrieval views, never replacements for the user's query.

Each rewrite records:

```text
origin: original | deterministic | model proposal | feedback | missing slot
stage: initial retrieval | reranking | iterative retrieval
token estimate
latency budget units
proposal status
```

Deterministic alias, identifier, path, symbol, entity, and date/version
expansions are ordered before model proposals. The full original query and
deterministic views may participate in initial retrieval. Constraint subqueries
are restricted to reranking, while missing-slot queries require a named,
non-empty missing evidence slot and belong only to iterative retrieval.
Model-generated proposals remain untrusted until plan, capability, scope,
security, freshness, snapshot, and budget validation succeeds.

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

This is a capability graph, not a mandatory fixed pipeline. Stages may be skipped when the plan and policy permit it; executable plans use the canonical order declared by the validator, and unsupported stage orderings are rejected before effects.

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

### Visual-document evidence

PDF parsing preserves page order and uses each page's MediaBox dimensions to normalize detected table/figure regions into typed structure nodes. Text-bearing pages expose page evidence; detected layout regions expose exact PDF coordinates and retain the immutable PDF blob as their source snapshot. Pages with missing or unreliable text extraction return `NeedsOcr` and may retain only layout metadata; they never emit fabricated text evidence. Region candidates use `SourceLocation::Region` and pass the same scope, ACL, trust, sensitivity, quarantine, and prompt-injection checks as text candidates.
Optional `visual_page_v1` retrieval is a separate image-modality lane with its own provider fingerprint and index generation. It accepts only visual representations with PDF page/region provenance. When no visual provider is configured or its identity is stale, the search trace records a text/layout degradation instead of silently relabeling text embeddings or losing coordinates.
When configured, bounded multimodal reranking may reorder only those page/region candidates using a local, no-retention visual provider. Text and layout candidates remain in the result, PDF region coordinates stay attached to their evidence, and input, score, output, and latency caps are enforced. Provider, privacy, vector, or budget failures return the original text/layout ranking with an explicit reranking fallback status; no visual model is mandatory.
Visual activation is benchmark-gated by frozen `Text`, `Table`, `Chart`, `Figure`, `Formula`, and `ScannedPage` query classes. The text/layout route remains the default; a visual promotion record may activate visual retrieval and reranking only for classes that demonstrate page/region quality and citation-alignment gains within latency, memory, disk, energy, privacy, and security budgets.

### Repository Code Intelligence

The deterministic repository lane is a persisted projection, not an embedding fallback.
`maestria index repository <path>` discovers every Cargo workspace under the repository
root with a bounded walk (skipping `.git`, `target/`, hidden directories, and
privacy-excluded paths) and records each workspace's packages, targets, features,
dependencies, and Rust symbols into one repository-wide index. Python repositories
(PEP 621 `pyproject.toml`, `setup.cfg`, or `setup.py`) are discovered with the same
walk: each distribution becomes a package whose targets are its top-level packages
and modules, and Python classes, functions, methods, imports, and calls are extracted
with a deterministic tokenizer (no execution, no installation). Each symbol carries
repository root, commit SHA, worktree identity, source path/range, and parser generation.

Exact queries remain available without neural indexes:

```bash
maestria search code symbol "RetrievalEngine"
maestria search code path "crates/ecosystem/maestria-retrieval"
maestria search code regex "impl .*CandidateRetriever"
maestria search code doc "Build a fresh index"
maestria search code markers todo
maestria search code markers unsafe
maestria search code changed
maestria search code changed --since HEAD~1
```

`search code doc <pattern>` matches symbols whose doc comment contains the
pattern as a case-sensitive substring. Doc text comes from `#[doc]`
attributes — `///` lines, `//!` file/module docs, and explicit
`#[doc = "…"]` — joined and trimmed deterministically from the AST, never
from a model. File-level `//!` docs attach to the file's root module symbol.

`search code markers <kind>` matches symbols carrying a source marker, where
`kind` is one of `todo`, `fixme`, `hack`, or `unsafe` (case-insensitive;
anything else is a parse error). todo/fixme/hack markers come from a
deterministic raw-text comment scan (strings and char literals are skipped);
each marker attaches to the innermost symbol whose source range contains the
comment, with an orphan comment attaching to the file's root module symbol.
Markers carry validated one-based inclusive source ranges and are never
LLM-derived. `unsafe` matches `UnsafeBlock` symbols and unsafe-bearing
declarations instead of comment markers.

`search code changed` returns symbols in files that changed since the
indexed baseline (the persisted delta), and `--since <commit>` computes the
change set live: `git diff --name-only <commit> HEAD` plus the current
porcelain dirty set. `--since` accepts a full 40-hex SHA-1, a short hex
prefix, or a `HEAD`-family reference (`HEAD`, `HEAD~2`, `HEAD^`); anything
else is rejected before any git call. Like every code query, a stale index
fails closed with the freshness message before current-state claims are
allowed.

Bounded repository context can expand exact/lexical seeds through typed relations:

```bash
maestria search code context "RetrievalEngine" --depth 2 --nodes 64 --direction both
```

The index is JSON-persisted under the instance system directory. Query results expose the
stored provenance and generation; a parser-generation mismatch or repository worktree
mismatch is an explicit freshness failure before current-state claims are allowed. The
repository projection may also carry deterministic AST relations for resolved definitions,
imports, calls, implementations, and tests. Relation endpoints retain their source records,
source spans, confidence, and parser generation. Bounded context queries traverse those
typed relations from exact/lexical seeds while preserving seed lineage, direction, depth,
node, and relation-kind caps. Missing LSP/provider support is recorded as an explicit
degraded status; unresolved edges are omitted rather than presented as facts. Live reads
and tests are separate governed effects and must return their own evidence.

`maestria index repository <path>` reports its build mode: `mode=full` (from scratch),
`mode=incremental` (only files whose extraction inputs changed are re-parsed, and the
result is exactly equivalent to a full rebuild at the same repository state), or
`mode=noop` (index already current; nothing written). The summary it prints includes a
`changed` section (`changed_files=N changed_symbols=M` plus the persisted JSON delta):
the porcelain dirty set (staged plus worktree edits) unioned with
`git diff --name-only <baseline> HEAD`, where the baseline is the commit of the
replaced index for incremental rebuilds and the empty set for from-scratch full
builds. Both porcelain status and the name-only diff are git metadata calls — no file
contents are read to compute the delta, so a clean worktree costs the same as before.
`changed.symbols` are the record ids of indexed symbols whose file is in the changed
set, ordered by file then qualified name. Worktree identity is derived from
git without content reads when the worktree is clean (index blob map plus porcelain
status); dirty, untracked, and ignored files are content-hashed. Every discovered
manifest (and its `Cargo.lock`) participates in the worktree identity, so editing a
nested manifest invalidates the index like any other source change. New cargo
auto-discovery targets and manifest changes fall back to a full rebuild. Sources are
registered as canonical artifacts through the kernel pipeline, so code queries authorize
symbols against durable, blob-verified evidence; files the kernel refuses to index
(secret-bearing content) are skipped by authorization rather than erroring.

Multiple workspace roots share one repository index: member manifests that resolve to an
already-indexed workspace are deduplicated, and packages are deduplicated by package id.
A repository without any supported manifest (neither `Cargo.toml` nor a Python manifest)
indexes to a valid, fresh empty index: `mode=full` with zero symbols, no matches from
code queries, and no-op rebuilds on subsequent runs. A root `Cargo.toml` that exists but
fails `cargo metadata` is a typed error, as is a root `pyproject.toml` that fails to
parse. A NESTED manifest that fails does not kill the index: the broken workspace or
distribution is skipped, the healthy ones are still indexed, and the degradation is
surfaced as a `workspace_warnings` entry in the summary JSON plus a `warning:` line on
stderr — never silently.

Repository-code promotion is governed by the frozen `rust-repository-frozen-v1` and
`python-repository-frozen-v1` benchmarks in `maestria-retrieval`. They compare the
Phase C route with the code-specialized route for exact-span recall, evidence-chain
accuracy, p95 latency, freshness errors, outcome accuracy, abstention accuracy, peak
memory, privacy violations, security violations, and energy across seven query classes:
exact symbol, definition/reference, issue-to-file, multi-hop dependency, test
association, stale worktree, and correct abstention.
Specialized routing is shadowed by default. A promotion record may activate it
only for classes that meet the material quality delta, latency budget, freshness,
and abstention gates; all other classes remain on Phase C. Stale indexes produce
stale evidence or an explicit abstention, never an unverified current-state
claim.
The daemon and CLI constructors default to shadow mode; an operator may supply the
typed promotion record returned by the benchmark comparison to the explicit
repository-policy runtime constructor. No persisted promotion file is trusted
automatically, so an absent or unverifiable promotion remains on Phase C.

### Build latency

`index repository` registers each symbol-bearing source as a canonical kernel
artifact through a bounded submit-ahead window: at most 4 artifacts are in
flight (submitted but not yet awaited) at any time, and waits for the
terminal indexed state are serialized oldest-first. The window is sized
inside the runtime's effect-semaphore headroom (16 slots) so a mid-size
repository never floods the input loop; it is a named constant
(`REGISTRATION_IN_FLIGHT`) in `crates/apps/maestria-cli/src/commands/
code_intel_sources.rs` and must not be raised without fresh measurements of
the runtime pipeline. Per-artifact runtime cost is dominated by the
full-text indexing effect (tantivy commit per artifact across cards, lexical
cards, chunks, and lexical chunks); commit batching in the runtime is
intentionally out of scope until it is proven safe with the daemon e2e.

Build latency is measured by `repository_build_latency_tests` in
`maestria-retrieval`: it generates fixture workspaces (50 and 200
symbol-bearing files), times cold full builds of the extraction pipeline
over several runs, and writes p50/p95 latency to
`target/benchmark-reports/repository-build-latency.json`, which the
`benchmark_evidence_v1` ledger validates so latency regressions block
promotion (Rule 44).

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