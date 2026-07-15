# Maestria Architecture

## 1. Purpose and Scope

Maestria is a governed system for maintaining auditable domain state from source observations, evidence, claims, memory, decisions, tasks, and validation results.

This document defines:

- system identity and ownership boundaries;
- dependency direction;
- domain purity and effect handling;
- governance and runtime responsibilities;
- adapter and projection boundaries;
- typed, budgeted search;
- evidence and validation invariants;
- DTO and persistence rules.

It does not replace the normative requirements in [`docs/SPECS.md`](SPECS.md) or the design principles in [`docs/PHILOSOPHY.md`](PHILOSOPHY.md).

Specific storage engines, search indexes, models, parsers, and harness implementations are replaceable until benchmarked against Maestria’s versioned evaluation sets.

---

## 2. System Identity

Maestria has four architectural concerns:

| Concern | Responsibility |
|---|---|
| Domain kernel | Defines valid state, transitions, events, and effects |
| Governance | Determines whether proposed effects and transitions are permitted |
| Runtime | Executes effects, coordinates asynchronous work, and returns results |
| Adapters and projections | Connects external systems and creates queryable representations |

The domain kernel owns **authoritative state integrity**, not external factual truth.

A source produces an observation. Evidence preserves that observation. A claim represents a normalized but potentially uncertain proposition. Memory promotes useful claims under policy. Decisions select actions based on evidence and policy. Validation assesses whether support is sufficient.

Maestria may record that a source says something, that evidence is fresh, or that a task passed a validation procedure. It cannot make an external claim true.

### 2.1 Domain State and External Truth

| Domain state | External factual truth |
|---|---|
| Artifact and version identities | Whether an artifact accurately describes reality |
| Immutable evidence references | Whether the source is correct |
| Claims and their uncertainty/status | Whether a claim is objectively true |
| Memory promotion and deprecation state | Whether promoted information remains current |
| Task and validation state | Whether the validated result generalizes beyond its scope |
| Conflict, freshness, and trust annotations | Whether conflicting sources can be resolved conclusively |

External observations must retain provenance, scope, and freshness. Contradictions and unsupported claims remain representable.

---

## 3. Architectural Dependency Direction

The dependency direction is inward toward the domain:

```text
Application crates
    ↓
Runtime
    ↓
Governance and domain ports
    ↓
Domain kernel

Adapters and projections implement ports and are invoked by runtime.
```

The domain kernel must not depend on infrastructure.

### 3.1 Dependency Rules

1. Domain code does not import storage, networking, filesystem, async runtime, search-engine, model-provider, or harness implementations.
2. Governance may inspect domain state and produce policy decisions, but does not perform I/O.
3. Runtime may execute effects and submit results, but may not mutate domain state directly.
4. Adapters may depend on provider-specific types internally, but those types do not cross domain or port boundaries.
5. Applications compose services; they do not implement domain rules or policy shortcuts.
6. Projections are rebuildable representations, not independent authorities over domain meaning.

---

## 4. Component Ownership

The following boundaries are logical contracts. Crate names may change without changing the architecture.

| Component | Owns | Must not own |
|---|---|---|
| `maestria-domain` | Domain entities, state, transitions, events, effects, strong identifiers | I/O, async workers, SQL, prompts, provider clients |
| `maestria-governance` | Scope, risk, approval, validation, freshness, trust, memory, and security policy | I/O or effect execution |
| `maestria-runtime` | Effect execution, workers, queues, cancellation, retries, journaling, supervision | Direct domain mutation |
| Storage adapter | Current queryable state and event persistence | Domain interpretation |
| Blob adapter | Immutable source snapshots, logs, reports, evidence packs | Mutable policy state |
| Parser adapters | Source parsing and document structure | Direct persistence |
| Retrieval subsystem | Search planning, candidate generation, fusion, reranking, expansion, evidence packing | Domain ownership or final factual judgment |
| Memory subsystem | Candidate deduplication, promotion workflow, staleness, contradiction, deprecation | Unapproved memory promotion |
| Validation subsystem | Validation runners and reports | Unverified completion |
| Harness subsystem | Normalized external execution and capability reporting | Memory writes or task finalization |
| Application crates | Composition, transport, CLI, configuration | Domain logic, direct SQL mutations, policy bypasses |

---

## 5. Domain Kernel

`maestria-domain` is the authoritative implementation of domain meaning.

It owns types including:

```text
Artifact, ArtifactVersion, ContentHash
Chunk, Card
Claim, Relation
Evidence, EvidenceSpan
MemoryCandidate, Memory
Task, TaskState, TaskTransition
DomainEvent, DomainInput, MaestriaEffect
ValidationReport shape
PolicyDecision shape
InstanceManifest
HarnessCapabilityDescriptor
```

Domain identifiers use strong types rather than interchangeable strings:

```rust
pub struct ArtifactId(Uuid);
pub struct EvidenceId(Uuid);
pub struct ClaimId(Uuid);
pub struct MemoryId(Uuid);
pub struct TaskId(Uuid);
pub struct EventId(Uuid);
pub struct InstanceId(String);
```

### 5.1 Transition Model

The preferred domain API is:

```rust
pub struct Transition {
    pub events: Vec<DomainEvent>,
    pub effects: Vec<MaestriaEffect>,
}

pub trait DomainReducer {
    fn apply(
        input: DomainInput,
        state: &mut DomainState,
    ) -> Result<Transition, DomainError>;
}
```

The reducer:

- validates inputs against current state;
- applies only valid state transitions;
- emits domain events;
- emits declarative effects;
- performs no external I/O;
- does not sample wall-clock time or randomness internally.

Time, randomness, external outputs, and provider responses are inputs to the transition process when they affect state.

### 5.2 State Mutation Invariant

The only supported path for domain state mutation is:

```text
DomainInput
  → DomainReducer
  → DomainState transition
  → DomainEvent and MaestriaEffect
```

Runtime, storage, search, harness, and application code must not mutate domain state through side channels.

---

## 6. Effects and Runtime Execution

A `MaestriaEffect` is a declarative request to perform work outside the domain kernel.

Examples include:

```text
SearchKnowledge
ReadArtifact
FetchWebEvidence
RunHarnessAction
RunValidation
PersistBlob
BuildProjection
```

The domain normally emits one task-significant effect for a cohesive operation. An adapter may perform multiple internal stages without exposing each implementation detail as domain state.

An operation becomes a separate effect when it crosses a policy, approval, trust, scope, or side-effect boundary.

### 6.1 Runtime Responsibilities

`maestria-runtime` owns:

- effect execution;
- bounded queues and backpressure;
- worker supervision;
- cancellation;
- retry policy;
- logical epochs and generation identifiers;
- stale-result rejection;
- adapter lifecycle;
- transition journaling;
- shutdown coordination.

Runtime behavior is:

```text
Effect
  → adapter execution
  → normalized result
  → DomainInput
  → domain reducer
```

A result from an obsolete task generation must be rejected rather than applied to current state.

---

## 7. Governance

Governance is policy, not execution.

It provides composable capabilities for:

```text
scope and authorization
risk classification
approval
completion validation
memory promotion
freshness
conflict handling
prompt-injection boundaries
secret handling
web access
autonomy limits
```

Representative capability contracts are:

```rust
pub trait ClassifyRisk {
    fn classify(&self, effect: &MaestriaEffect, scope: &Scope) -> RiskClass;
}

pub trait DecideApproval {
    fn decide(&self, risk: RiskClass, profile: AutonomyProfile) -> PolicyDecision;
}

pub trait ValidateCompletion {
    fn validate(&self, task: &Task, evidence: &[Evidence])
        -> ValidationRequirement;
}
```

Governance may inspect domain state and reject, approve, constrain, or require review of an effect. It must not execute the effect.

---

## 8. Source, Evidence, Claims, and Memory

The epistemic flow is:

```text
Source
  → observation
  → evidence
  → claim
  → memory candidate
  → governed memory
  → decision or validation
```

### 8.1 Required Properties

- Source snapshots and evidence spans are immutable.
- Trust, freshness, conflict, and validity annotations are versioned.
- Claims may be unsupported, stale, contradicted, superseded, or disputed.
- Memory promotion requires evidence and governance.
- Search snippets are candidates, not evidence.
- Generated summaries and contextual retrieval text never replace raw source evidence.
- A validation result proves only what its method and scope support.

---

## 9. Typed, Budgeted Search

Search is a planned capability, not a fixed vector-query pipeline.

Every non-trivial search produces a typed plan containing:

```text
query intent
corpus and trust scope
corpus/index snapshot
freshness requirement
modalities
candidate retrievers
fusion and reranking policy
context expansion policy
resource and quality budgets
stop conditions
required evidence coverage
```

```rust
pub struct SearchPlan {
    pub query_id: QueryId,
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

A model may propose a plan. Runtime validates it against:

- available capabilities;
- schema;
- policy and scope;
- corpus and index generations;
- budgets;
- freshness requirements;
- trust boundaries.

Runtime owns execution.

### 9.1 Search Outcome

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

pub struct SearchOutcome {
    pub evidence: Vec<EvidenceCandidate>,
    pub coverage: EvidenceCoverage,
    pub conflicts: Vec<ConflictSet>,
    pub trace: SearchTraceId,
}
```

Retrieval providers remain behind adapters. Search plans, outcomes, and traces are typed boundary objects.

### 9.2 Search Invariants

1. Every candidate maps to a source artifact version and evidence span.
2. Scope, ACL, trust, sensitivity, quarantine, and freshness filters are applied before candidate generation, scoring, fusion, reranking, expansion, or evidence packing.
3. Incompatible model or index fingerprints are never compared.
4. Duplicate and source-independence information is retained.
5. Conflicting and counterevidence results are surfaced, not silently collapsed.
6. Top-k is a resource ceiling, not a completeness guarantee.
7. Search may stop with incomplete evidence, conflict, or abstention.
8. The trace records plans, rewrites, stages, scores, filters, expansions, budgets, and stop reasons.
9. A search trace explains retrieval behavior but is not authoritative domain state.

Search implementations are replaceable until evaluated on Maestria’s versioned query set. No model name, public leaderboard, architecture diagram, or backend selection proves that retrieval works for Maestria.

---

## 10. Document and Retrieval Projections

Source material is preserved before it is projected into retrieval units:

```text
source artifact
  → artifact version
  → document structure
  → retrieval representations
  → indexes and evidence spans
```

A structure tree may contain:

```text
document/page
section/subsection
paragraph/list/table
figure/caption/region
code module/symbol/test
web heading/code block
```

Nodes retain source offsets, coordinates, parentage, section paths, parser generation, modality, and content hashes.

There is no universal chunk size. Implementations may use structural chunks, summaries, propositions, symbols, table entries, visual regions, or other representations, provided each derived unit retains exact source lineage.

Representations are distinct:

```text
raw source text       exact evidence and citation
retrieval text        normalized search representation
contextual text       structure-enriched retrieval representation
summary text          optional generated projection
visual representation optional visual retrieval input
```

Generated representations are rebuildable and never authoritative over raw evidence.

---

## 11. Adapters, Projections, and Index Generations

Storage and retrieval systems are replaceable implementations of contracts.

Logical stores may include:

```text
current state
event log
artifact registry
evidence
memory
blobs
full-text index
vector or other similarity index
graph projection
web snapshots
```

The architecture does not require a particular database, filesystem, search engine, vector engine, model, or algorithm.

### 11.1 Projection Rules

- Metadata and current state are queryable through a storage contract.
- Large immutable content is addressed by content hash.
- Indexes and graph structures are projections.
- Projections can be rebuilt from authoritative source state and snapshots.
- Each index generation records corpus snapshot, schema, model, preprocessing, and representation fingerprints.
- Activation is atomic.
- Previous active generations remain available for rollback until validated.
- Old representations are never reinterpreted under a new fingerprint.

Implementations are replaceable until benchmarked for quality, latency, resource use, correctness, migration, and recovery.

---

## 12. DTO Boundaries

Domain types, persistence rows, API responses, and harness payloads are different contracts.

```text
Domain type != database row != API response != provider payload
```

Adapters map explicitly:

```text
ArtifactRow          → Artifact
EvidenceRow          → Evidence
DomainEventRow       → DomainEvent
TaskRow + event rows → Task
Blob manifest        → BlobRef
Harness DTO          → HarnessOutcome
API request          → DomainInput or validated application command
```

Provider-specific request and result types must not cross into the domain or shared ports.

---

## 13. Validation and Completion

Validation is a completion gate, not a decorative report.

Validation may check:

```text
citation alignment
evidence existence
source freshness
task state
command exit status
test execution
diff scope
secret exposure
policy compliance
harness capability and scope
search coverage
conflict reporting
retrieval regression
```

A task may be `CompletedWithWarnings` when limitations are explicit. It cannot be `CompletedVerified` when required evidence is missing, invalid, stale beyond policy, or contradicted without resolution.

Validation reports record:

```text
task and claim checked
method
commands or tests run
evidence identifiers
harness run identifiers
pass/fail/warn status
freshness and policy status
known gaps
```

A validation report establishes the result of a bounded procedure. It does not establish unrestricted external truth.

---

## 14. Harness Boundary

Harnesses expose normalized capabilities:

```rust
pub trait HarnessAdapter: Send + Sync {
    async fn capabilities(&self) -> Result<HarnessCapabilities, HarnessError>;

    async fn execute(
        &self,
        request: HarnessRequest,
    ) -> Result<HarnessOutcome, HarnessError>;
}
```

An outcome includes:

```text
run identifier
capability used
scope checked
command or action
structured result or stdout/stderr
duration
exit status
created artifacts
diff summary
validation hints
```

Harnesses:

- operate only within granted scope;
- never write memory directly;
- never finalize tasks;
- return observations to runtime;
- rely on governance and validation for authorization and interpretation.

---

## 15. Observability, Replay, and Evaluation

The system records:

- transition journals;
- domain events;
- effect and adapter outcomes;
- search traces;
- validation reports;
- index and model fingerprints;
- artifact and corpus snapshots;
- policy decisions;
- generation identifiers.

Given the same initial state and ordered `DomainInput` stream, domain replay must produce the same events and final state. Runtime timestamps and external outputs are replay inputs, not hidden reducer behavior.

Every material retrieval change must be evaluated against a versioned Maestria-specific query set. Evaluation covers retrieval quality, evidence coverage, citation alignment, abstention, conflict handling, security boundaries, latency, resource use, and migration behavior.

External benchmarks may supplement this evaluation, but cannot replace it.

---

## 16. Explicit Invariants

1. **Domain purity** — The domain kernel performs no I/O, scheduling, provider calls, or hidden nondeterministic sampling.
2. **Single mutation path** — Domain state changes only through validated reducer inputs.
3. **Effect separation** — Effects describe work; runtime executes work.
4. **Policy separation** — Governance authorizes and constrains; adapters execute.
5. **Truth boundary** — Maestria preserves and evaluates observations; it does not make external claims true.
6. **Evidence lineage** — Derived retrieval units and generated summaries retain exact source lineage.
7. **Immutable evidence** — Source snapshots and evidence spans are immutable; annotations are versioned.
8. **Freshness visibility** — Stale, missing, quarantined, and conflicting sources remain explicit.
9. **Search budgeting** — Every non-trivial search has validated scope, budgets, coverage requirements, and stop conditions.
10. **Provenance** — Every final evidence item identifies its source version and span.
11. **Fingerprint compatibility** — Representations are compared only within compatible model and index generations.
12. **Projection replaceability** — Indexes and storage projections may be replaced without changing domain meaning.
13. **Validation gating** — Required evidence and validation must pass before verified completion.
14. **DTO isolation** — Provider, storage, API, harness, and domain types do not substitute for one another.
15. **Benchmark requirement** — No implementation is a permanent default until it is benchmarked against Maestria’s requirements.