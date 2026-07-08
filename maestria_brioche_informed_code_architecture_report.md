# Maestria Code Architecture Report — Brioche-Informed Revision

## Rust-Native Local Brain with Enforceable Philosophy, Deterministic Kernel, Explicit Effects, and Swappable Harnesses

**Status:** Updated architecture report informed by Brioche repository analysis  
**Project family:** Brio / Brioche / Maestria  
**Primary source draft:** `/home/arobin/Téléchargements/maestria_updated_code_architecture_report(3).md`  
**Primary reference implementation:** `/home/arobin/Dev/Brioche`  
**Maestria repository state observed:** `/home/arobin/Dev/maestria` currently contains only `LICENSE`; this report treats Maestria as a greenfield implementation target.  
**Date:** 2026-07-08

---

## 0. Executive Decision

Maestria should keep the previous report's product identity: **a local-first second brain and task-resolution layer for AI agents with a swappable harness boundary**.

The main update from Brioche is not a new database, UI, or model provider. The main update is architectural discipline:

```text
A good Maestria repository must have an enforceable code philosophy,
a deterministic domain kernel,
a strict mechanism/policy split,
explicit declarative effects,
typed evidence and event contracts,
bounded async runtime bridges,
contract tests for adapters,
and CI gates that make drift impossible to ignore.
```

The previous report already says Maestria should be Rust-first, local-first, contract-first, source-grounded, and policy-gated. Brioche proves how to make those words operational:

- `docs/PHILOSOPHY.md` is a CI-enforced canon, not aspirational prose.
- `docs/SPECS.md` is a canonical invariant ledger and architecture book.
- `brioche-core` is a synchronous pure kernel that emits `Vec<Effect>` instead of performing I/O.
- `brioche-shell-runtime` owns async I/O, bounded channels, backpressure, watchdogs, effect execution, and loopback inputs.
- `brioche-governance-default` keeps policy outside mechanism.
- `brioche-macro` turns documentary rules into compile-time checks.
- `.github/workflows/ci.yml`, `scripts/philosophy-check.py`, custom lints, clippy, docs, proptests, profiles, and benchmarks enforce the doctrine.

The Maestria architecture should therefore be revised from:

```text
modular local-first second brain with ports/adapters
```

to:

```text
modular local-first second brain with a pure deterministic domain kernel,
explicit effect inversion, a policy/governance layer, swappable harness shell,
and an enforceable repository philosophy.
```

---

## 1. What Brioche Adds to the Existing Maestria Draft

The source Maestria draft already contains strong ideas:

- Maestria owns memory, evidence, policy, task state, validation, and workspace organization.
- Harnesses own shell/browser/filesystem/tool execution.
- The model does not own truth.
- Storage should be local-first: SQLite, Tantivy, sqlite-vec, filesystem blobs, event logs.
- Components should sit behind stable Rust traits and adapter contracts.
- Validation should gate task completion.
- Autonomy must be scoped.

Brioche adds the missing implementation doctrine.

### 1.1 Documentation Is Part of the Architecture

Brioche's `docs/PHILOSOPHY.md` starts with an enforceability claim and is wired into CI. This is the most important improvement for Maestria.

Maestria needs the same repository-level documents:

```text
docs/
  PHILOSOPHY.md         # enforceable code canon
  SPECS.md              # canonical architecture + invariant book
  architecture/
    book-i-domain.md
    book-ii-governance.md
    book-iii-runtime.md
    book-iv-ecosystem.md
  adr/
    ADR-0001-modular-monolith.md
    ADR-0002-local-first-storage.md
    ADR-0003-harness-boundary.md
    ADR-0004-effect-inversion.md
  invariants/
    matrix.md
```

`PHILOSOPHY.md` must not be a README-style manifesto. It must define rules that CI can check:

- forbidden panic paths in the domain kernel;
- no hidden I/O in domain code;
- invariant references on public hot-path items;
- documentation density rules;
- module cohesion rules;
- no bare TODO/FIXME;
- no stale backup/generated artifacts in source paths;
- no adapter-specific types crossing inward;
- bounded async channel rules;
- cancellation documentation for public async functions;
- contract-test obligations for every adapter.

### 1.2 Mechanism and Policy Must Be Separate in Code, Not Only in Text

The existing report says Maestria has policy and validation. Brioche shows the stronger rule:

```text
Mechanism owns state transitions.
Policy owns decisions.
Policy changes must not modify mechanism types.
```

For Maestria:

- `maestria-domain` owns artifacts, evidence, claims, tasks, memory candidates, events, validation status, and mechanical transitions.
- `maestria-governance` / `maestria-policy` owns approval rules, autonomy profiles, scope checks, risk classes, freshness rules, memory promotion rules, and validation policies.
- `maestria-runtime` executes effects and sends results back as typed inputs.
- `maestria-harness-*` adapters provide capabilities but do not own task truth, evidence authority, policy, or memory.

### 1.3 Effects Should Be Values

Brioche's core emits `Vec<Effect>` and the shell executes them. This is the cleanest model for Maestria.

Maestria should not let services directly call random adapters from deep in workflows. It should represent side effects as typed values:

```rust
pub enum MaestriaEffect {
    PersistEvent(DomainEvent),
    StoreBlob(StoreBlobRequest),
    IndexFullText(IndexRequest),
    EmbedChunks(EmbeddingRequest),
    QueryHarness(HarnessRequest),
    FetchWeb(WebFetchRequest),
    RunValidation(ValidationRequest),
    RequestApproval(ApprovalRequest),
    EmitDiagnostic(DiagnosticEvent),
}
```

Effects are not actions. They are **declarative intentions**. Runtime interprets them.

This gives Maestria:

- replayable task transitions;
- auditable autonomy;
- one policy gate before side effects;
- deterministic domain tests without mocks;
- swappable adapters without changing domain transitions;
- a clean failure model when an adapter is unavailable.

### 1.4 Determinism Is a Design Constraint

Brioche bans nondeterminism from the core: no hidden randomness, no clocks, no unordered persisted collections, no implicit I/O. Maestria needs a scoped version of that rule.

Maestria's domain kernel must be deterministic given:

```text
current state + input event + policy profile + indexed evidence snapshot
```

The runtime may use clocks, web requests, file watchers, and harness calls, but those must enter the domain as typed inputs:

```rust
pub enum DomainInput {
    UserIntent(UserIntent),
    ArtifactDetected(ArtifactDetected),
    ParserCompleted(ParserResult),
    SearchCompleted(SearchResultSet),
    HarnessRunCompleted(HarnessRunResult),
    ValidationCompleted(ValidationReport),
    ApprovalResolved(ApprovalDecision),
    ClockTick(LogicalTick),
}
```

The rule:

```text
The domain never samples the world.
The runtime samples the world and reports typed facts.
```

### 1.5 Repository Enforcement Must Exist on Day One

The old Maestria report mentions CI near the end. Brioche makes CI part of the architecture.

Maestria's MVP should include enforcement before feature breadth:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps with RUSTDOCFLAGS=-D warnings
cargo test --workspace --all-features
cargo test --workspace --doc --all-features
cargo deny check all
cargo machete
custom philosophy checker
custom invariant-reference checker
contract-test suite
profile cargo check matrix
benchmark compile smoke for hot paths
```

This is not process overhead. It preserves coherence as the project grows.

---

## 2. Updated Product Boundary

The boundary remains the same, but the language should be stricter.

### 2.1 Maestria Owns

```text
- domain truth
- artifact registry
- evidence ledger
- claim and relation model
- memory candidates and promoted memories
- task state machine
- validation reports
- policy decisions
- autonomy profiles
- scope rules
- event log
- indexed projections
- workspace organization proposals
- durable diagnostic history
```

### 2.2 The Harness Owns

```text
- live filesystem access
- shell command execution
- browser automation
- screenshots and UI observation
- code editing mechanics
- external tool execution
- optional web search implementation
- live command output streams
```

### 2.3 The Runtime Owns

```text
- Tokio worker supervision
- bounded queues
- cancellation tokens
- backpressure
- retries
- timeouts
- watchdogs
- adapter lifecycle
- effect execution
- loopback of async results into the domain
```

### 2.4 The Model Owns

```text
- language understanding
- summarization proposals
- candidate extraction
- draft reasoning over evidence
- plan proposals
```

The model does **not** own:

```text
- truth
- policy
- validation status
- task completion status
- memory promotion
- side-effect permission
- evidence integrity
```

---

## 3. Updated High-Level Architecture

```text
                                  User
                                   │
                                   ▼
                         Selected Harness / UI
               Claude Code / Pi / Hermes / CLI / Desktop
                                   │
                                   ▼
                        Harness Adapter Boundary
                 capabilities + policy bridge + run logs
                                   │
                                   ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                         Maestria Runtime / Shell                           │
│                                                                            │
│  bounded queues │ cancellation │ backpressure │ watchdog │ effect executor │
│  file watchers  │ workers      │ adapter pool │ telemetry│ result loopback │
└───────────────────────────────────┬────────────────────────────────────────┘
                                    │ typed DomainInput / Vec<MaestriaEffect>
                                    ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                         Maestria Domain Kernel                             │
│                                                                            │
│  Artifact │ Evidence │ Claim │ Relation │ Event │ Task │ MemoryCandidate  │
│                                                                            │
│  Pure transitions. No I/O. No adapter types. No clocks. No hidden effects. │
└───────────────────────────────────┬────────────────────────────────────────┘
                                    │ policy hooks / decisions
                                    ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                         Maestria Governance Layer                          │
│                                                                            │
│  scope policy │ autonomy profile │ approval rules │ validation policy      │
│  memory promotion │ freshness/contradiction rules │ risk classification   │
└───────────────────────────────────┬────────────────────────────────────────┘
                                    │ ports
                                    ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                         Adapters and Projections                           │
│                                                                            │
│  SQLite metadata │ filesystem blobs │ Tantivy │ sqlite-vec │ graph edges    │
│  parsers         │ web fetchers     │ harnesses │ model providers         │
└────────────────────────────────────────────────────────────────────────────┘
```

Dependency direction:

```text
apps / daemon / CLI / API / UI
  ↓
runtime / shell
  ↓
adapters / projections
  ↓
ports / service contracts
  ↓
governance / policy contracts
  ↓
domain kernel
```

Concrete dependencies point inward. Runtime and adapters may depend on domain types. Domain must never depend on runtime, adapters, model providers, databases, filesystem, web, UI, or harness implementations.

---

## 4. Repository Philosophy Canon for Maestria

Maestria should create `docs/PHILOSOPHY.md` before substantial code lands.

Recommended core text:

```text
This document is enforceable. CI and review block violations.
```

### 4.1 Maestria Paradigm

Maestria should adopt a version of Brioche's algebraic systems design:

1. **Data first, behavior second.** Model artifacts, evidence, task states, memories, policies, and effects as ADTs before writing services.
2. **Types are the spec.** Illegal task transitions, unsupported evidence kinds, unvalidated memories, and unscoped actions should be unrepresentable where practical.
3. **Traits are capabilities, not taxonomies.** A trait says `can search`, `can store blob`, `can run harness request`; it does not define an inheritance tree.
4. **Effects are explicit.** Domain transitions return effects; runtime performs them.
5. **Evidence is immutable.** Derived claims and memories point back to evidence spans, snapshots, command outputs, or validation reports.
6. **Policy is injected.** Autonomy profile changes must not mutate domain mechanism types.
7. **Validation gates truth.** A task cannot transition to verified completion without a validation report.

### 4.2 Non-Negotiable Rules

```text
- No hidden I/O in maestria-domain.
- No Tokio in maestria-domain.
- No SQLx, Tantivy, reqwest, filesystem I/O, or harness-specific code in maestria-domain.
- No direct adapter calls from policy code.
- No direct database mutation from CLI/API handlers.
- No untyped serde_json::Value holes in domain effects when the domain shape is known.
- No memory promotion without evidence references.
- No task completion without validation status.
- No harness action without policy classification.
- No unbounded internal channels.
- No bare TODO/FIXME in production code.
- No panic/unwrap/expect in the domain kernel.
```

### 4.3 Documentation Rules

Every non-trivial public domain item should document:

```text
- purpose
- invariant references
- complexity/budget if hot-path
- panic/error contract
- evidence/policy implications if relevant
```

Example:

```rust
/// Transitions a task from execution to validation pending.
///
/// # Invariants
/// - I-Task-ValidationGate: completed states require a validation report.
/// - I-Event-AuditTrail: every state transition emits a domain event.
///
/// # Complexity
/// O(1). Allocates only returned events/effects.
///
/// # Errors
/// Returns `DomainError::InvalidTransition` if the task is not executing.
pub fn request_validation(task: Task, request: ValidationRequest)
    -> Result<Transition<DomainEvent, MaestriaEffect>, DomainError>;
```

### 4.4 Documentation Density Rule

Do not copy Brioche's full doc verbosity blindly. Copy its **signal discipline**.

```text
Document invariants, edge cases, policy consequences, hot paths, and architecture rationale.
Do not add boilerplate to trivial accessors.
```

### 4.5 Module Cohesion Rule

Maestria should reject one-file-per-type fragmentation.

Recommended rule:

```text
Split by invariant owner and cohesive concern, not by type count.
Small files under ~60 lines of actual logic should be merged unless they isolate a public boundary.
Large files over ~400 lines of logic require a cohesion reason.
Files over ~900 physical lines require a checked exemption.
```

---

## 5. Canonical Specifications and Invariant Ledger

Brioche has `docs/SPECS.md` as a canonical architecture book. Maestria needs the same because its domain will otherwise become vague.

Recommended structure:

```text
# Project Maestria: Architectural Specifications

BOOK I — DOMAIN KERNEL
  Chapter 1: Foundations
  Chapter 2: Fundamental types
  Chapter 3: Domain inputs and effects
  Chapter 4: Task automaton
  Chapter 5: Evidence and memory contracts
  Chapter 6: Limits of the domain layer

BOOK II — GOVERNANCE AND POLICY
  Chapter 1: Policy principles
  Chapter 2: Autonomy profiles
  Chapter 3: Scope policy
  Chapter 4: Validation policy
  Chapter 5: Memory promotion policy
  Chapter 6: Interface contract with runtime and domain

BOOK III — RUNTIME AND SHELL
  Chapter 1: Effect loop and worker supervision
  Chapter 2: Backpressure and bounded channels
  Chapter 3: Cancellation and timeouts
  Chapter 4: Harness gateway
  Chapter 5: Watchdog and recovery
  Chapter 6: Limits of runtime

BOOK IV — STORAGE, PROJECTIONS, AND ECOSYSTEM
  Chapter 1: SQLite source of current state
  Chapter 2: Append-only event log
  Chapter 3: Blob store
  Chapter 4: Search/vector/graph projections
  Chapter 5: Parser/provider/tool adapters

BOOK V — INVARIANTS AND VERIFICATION
  Chapter 1: Domain invariants
  Chapter 2: Evidence invariants
  Chapter 3: Task invariants
  Chapter 4: Governance invariants
  Chapter 5: Runtime invariants
  Chapter 6: Storage invariants
  Chapter 7: Error taxonomy
  Chapter 8: Verification strategy
  Chapter 9: Compilation profiles
  Chapter 10: Replay and determinism
```

### 5.1 Initial Invariant Set

Maestria should name invariants from day one. Suggested starting set:

| Invariant | Rule |
|---|---|
| `I-Domain-Pure` | Domain transitions perform no I/O and sample no clocks. |
| `I-Domain-NoPanic` | Domain errors return typed errors or failure states; no panic path. |
| `I-Effect-Explicit` | All side effects are represented as `MaestriaEffect`. |
| `I-Event-AuditTrail` | Important state changes emit append-only domain events. |
| `I-Evidence-Immutable` | Evidence snapshots are immutable and content-addressed. |
| `I-Evidence-Provenance` | Claims, memories, and answers cite evidence spans/snapshots. |
| `I-Memory-CandidateGate` | LLM output can create candidates, not promoted memory. |
| `I-Memory-SourceBacked` | Promoted memory requires evidence and promotion policy. |
| `I-Task-StateMachine` | Task states transition only through domain functions. |
| `I-Task-ValidationGate` | Verified completion requires a validation report. |
| `I-Policy-BeforeAction` | Risky effects require policy decision before runtime execution. |
| `I-Harness-NoTruth` | Harness adapters execute and report; they do not own truth. |
| `I-Runtime-BoundedChannels` | Internal runtime channels are bounded and documented. |
| `I-Runtime-CancelSafe` | Public async runtime operations document cancellation behavior. |
| `I-Storage-ProjectionOnly` | Search/vector/graph stores are projections, not truth owners. |
| `I-Adapter-ContractTested` | Every adapter passes the shared behavior contract. |
| `I-Security-PromptUntrusted` | Indexed/web content is evidence, never instructions. |
| `I-Scope-ExplicitAutonomy` | Autonomous action is limited to explicit scope and profile. |

---

## 6. Revised Workspace Layout

The previous report proposed many crates. Brioche suggests grouping by architectural book and avoiding too many crates too early.

### 6.1 MVP Workspace

```text
maestria/
  Cargo.toml
  rustfmt.toml
  clippy.toml
  deny.toml
  README.md
  CONTRIBUTING.md

  docs/
    PHILOSOPHY.md
    SPECS.md
    architecture/
      book-i-domain.md
      book-ii-governance.md
      book-iii-runtime.md
      book-iv-storage-ecosystem.md
    adr/
      ADR-0001-modular-monolith.md
      ADR-0002-local-first-storage.md
      ADR-0003-harness-boundary.md
      ADR-0004-effect-inversion.md
    invariants/
      matrix.md

  scripts/
    philosophy-check.py

  crates/
    kernel/
      maestria-domain/
      maestria-governance/
    runtime/
      maestria-runtime/
    storage/
      maestria-storage-sqlite/
      maestria-blob-fs/
      maestria-search-tantivy/
    ecosystem/
      maestria-parsers/
      maestria-retrieval/
      maestria-memory/
      maestria-validation/
    harness/
      maestria-harness/
      maestria-harness-cli/
    apps/
      maestria-cli/
      maestria-daemon/
      maestria-api/
    infra/
      maestria-lint-core/
      cargo-maestria-lint/
      maestria-docgen/

  tests/
    contracts/
    integration/
    property/
    replay/
    fixtures/
    golden/
```

### 6.2 Later Workspace Expansion

Add only when boundaries become real:

```text
crates/storage/maestria-vector-sqlite
crates/storage/maestria-graph-sqlite
crates/providers/maestria-provider-openai
crates/providers/maestria-provider-local
crates/harness/maestria-harness-claude-code
crates/harness/maestria-harness-browser
crates/ecosystem/maestria-code-intel
crates/ecosystem/maestria-web-evidence
crates/apps/maestria-desktop
```

Rule:

```text
A crate exists when it protects a dependency boundary, owns a stable invariant set,
or needs independent contract tests.
```

---

## 7. Crate Responsibilities

### 7.1 `maestria-domain`

The sacred kernel.

Owns:

```text
Artifact
ArtifactId
ContentHash
Chunk
Card
Claim
Relation
Evidence
EvidenceSpan
MemoryCandidate
Memory
Task
TaskState
TaskTransition
ValidationReport
PolicyDecision type, if purely domain-shaped
DomainEvent
DomainInput
MaestriaEffect
InstanceManifest
HarnessCapabilityDescriptor
```

Must not contain:

```text
SQL
filesystem reads
network requests
Tokio worker logic
Axum routes
Tantivy schemas
embedding provider clients
LLM prompts
harness-specific execution
wall-clock sampling
randomness
```

Preferred API shape:

```rust
pub struct Transition {
    pub events: Vec<DomainEvent>,
    pub effects: Vec<MaestriaEffect>,
}

pub trait DomainReducer {
    fn apply(input: DomainInput, state: &mut DomainState) -> Result<Transition, DomainError>;
}
```

### 7.2 `maestria-governance`

Owns policy as composable capabilities:

```text
ScopePolicy
AutonomyPolicy
RiskClassifier
ApprovalPolicy
ValidationPolicy
MemoryPromotionPolicy
EvidenceFreshnessPolicy
PromptInjectionPolicy
ConflictResolutionPolicy
```

Traits should be capabilities, not taxonomies:

```rust
pub trait ClassifyRisk {
    fn classify(&self, effect: &MaestriaEffect, scope: &Scope) -> RiskClass;
}

pub trait DecideApproval {
    fn decide(&self, risk: RiskClass, profile: AutonomyProfile) -> PolicyDecision;
}

pub trait ValidateCompletion {
    fn validate(&self, task: &Task, evidence: &[Evidence]) -> ValidationRequirement;
}
```

Governance may inspect domain state and produce policy decisions. It must not execute I/O.

### 7.3 `maestria-runtime`

Owns async orchestration:

```text
EffectExecutor
WorkerSupervisor
BackpressureRegulator
CancellationRegistry
TaskQueue
RuntimeEventBus
Watchdog
TransitionJournal
RetryPolicy
ShutdownCoordinator
```

Runtime must:

- execute `MaestriaEffect` values;
- map adapter outputs into `DomainInput`;
- keep channels bounded;
- stamp long-running operations with logical epochs or generation IDs;
- reject stale async results when a task generation has moved on;
- never mutate domain state except by submitting `DomainInput` to the domain reducer.

### 7.4 `maestria-storage-sqlite`

Owns current queryable state and append-only event persistence.

Recommended tables:

```text
instances
source_roots
artifacts
artifact_versions
chunks
cards
claims
relations
evidence
memories
memory_candidates
tasks
task_events
domain_events
validation_reports
harness_runs
approvals
```

SQLite is the local truth store for current state, but the domain types define truth shape.

### 7.5 `maestria-blob-fs`

Owns immutable content-addressed blobs:

```text
raw file snapshots
extracted text
web snapshots
PDF page text
command outputs
screenshots
validation logs
reports
```

Rule:

```text
Blob paths are derived from content hashes.
Metadata lives in SQLite.
```

### 7.6 `maestria-search-tantivy`

Owns full-text projection.

It implements a `FullTextIndex` port. It does not own artifacts, chunks, evidence, or memory.

### 7.7 `maestria-parsers`

Owns parser registry and parser implementations.

Initial parsers:

```text
Markdown
plain text
Rust source metadata
Cargo.toml/workspace metadata
PDF extracted text wrapper
HTML/readability snapshot wrapper
```

Parsers return typed parse outputs. They do not write storage directly.

### 7.8 `maestria-harness`

Defines normalized harness types:

```rust
pub trait HarnessAdapter: Send + Sync {
    async fn capabilities(&self) -> Result<HarnessCapabilities, HarnessError>;
    async fn execute(&self, request: HarnessRequest) -> Result<HarnessOutcome, HarnessError>;
}
```

Harness outcomes must include:

```text
run id
capability used
scope checked
command/action
stdout/stderr or structured result
duration
exit status
artifacts created
diff summary if files changed
validation hints
```

Harnesses never write memory directly and never finalize tasks.

### 7.9 App Crates

`maestria-cli`, `maestria-daemon`, and `maestria-api` wire services together.

They may use `anyhow` at the edge. They must not contain domain logic, direct SQL mutations, or policy shortcuts.

---

## 8. Domain Model Updates from Brioche

### 8.1 Effects and Inputs

Maestria should use a paired input/effect model.

```rust
pub enum DomainInput {
    UserAsked(UserIntent),
    SourceRootRegistered(SourceRoot),
    ArtifactDetected(ArtifactDetected),
    ArtifactParsed(ParseResult),
    SearchIndexed(IndexResult),
    EvidenceOpened(EvidenceOpenResult),
    HarnessCompleted(HarnessOutcome),
    WebFetchCompleted(WebEvidenceResult),
    ValidationCompleted(ValidationReport),
    ApprovalResolved(ApprovalDecision),
    MemoryReviewed(MemoryReviewDecision),
}
```

```rust
pub enum MaestriaEffect {
    PersistEvent(DomainEvent),
    PersistState(PersistStateRequest),
    StoreBlob(StoreBlobRequest),
    ParseArtifact(ParseRequest),
    IndexFullText(IndexRequest),
    IndexVector(VectorIndexRequest),
    UpdateGraph(GraphUpdateRequest),
    QueryHarness(HarnessRequest),
    FetchWeb(WebFetchRequest),
    RunValidation(ValidationRequest),
    RequestApproval(ApprovalRequest),
    EmitDiagnostic(DiagnosticEvent),
}
```

### 8.2 Task State Machine

The previous report's task state machine is good. It should become a domain automaton with typed transitions.

```text
Created
  → Scoped
  → ContextRetrieving
  → Planning
  → AwaitingApproval?       # optional policy gate
  → Executing
  → Validating
  → CompletedVerified
  → CompletedWithWarnings
  → Blocked
  → Failed
```

Rules:

```text
- `CompletedVerified` requires a validation report with status pass.
- `CompletedWithWarnings` requires validation report with warnings.
- `Failed` requires error evidence or failed validation.
- `Blocked` requires a blocking reason and attempted next action.
- No state change bypasses DomainEvent emission.
```

### 8.3 Memory State Machine

Memory should mirror the task discipline.

```text
Observation
  → MemoryCandidate
  → NeedsEvidence
  → NeedsReview
  → Promoted
  → Deprecated
  → Contradicted
  → Superseded
```

Rules:

```text
- LLM output can propose `MemoryCandidate`, never `Memory`.
- Promotion requires evidence and a promotion decision.
- Contradiction detection creates a new event; it does not silently overwrite memory.
- Deprecated memories remain auditable.
```

### 8.4 Evidence Model

Evidence must be immutable and addressable.

```rust
pub enum EvidenceKind {
    FileSpan { path: SourcePath, start_line: u32, end_line: u32, content_hash: ContentHash },
    PdfSpan { blob: BlobId, page_start: u32, page_end: u32 },
    WebSnapshot { url: Url, snapshot: BlobId, fetched_at: Timestamp },
    CommandOutput { harness_run: HarnessRunId, stream: OutputStream, blob: BlobId },
    TestResult { harness_run: HarnessRunId, status: TestStatus, log: BlobId },
    Diff { harness_run: HarnessRunId, patch_blob: BlobId },
    Validation { report_id: ValidationReportId },
}
```

No evidence kind should be a raw string if its shape is known.

---

## 9. Runtime and Harness Model Updates

Brioche's runtime architecture maps directly to Maestria's harness problem.

### 9.1 Pure Domain, Impure Runtime

```text
Domain: decides what should happen.
Runtime: performs it.
Harness: reaches outside Maestria.
Adapter: translates provider-specific behavior into Maestria types.
```

### 9.2 Bounded Channels

Every internal channel should document:

```text
capacity
drop policy
producer
consumer
backpressure behavior
what happens on shutdown
```

Example:

```text
channel: ingestion_queue
capacity: 1024 artifact jobs
drop policy: reject new low-priority jobs, never drop explicit user tasks
producer: file watcher / CLI index command
consumer: ingestion_worker
shutdown: persist pending queue to SQLite before exit if possible
```

### 9.3 Generation IDs / Epochs for Stale Results

Brioche uses epochs to reject stale async responses. Maestria needs the same pattern for tasks and harness runs.

```rust
pub struct TaskGeneration {
    pub task_id: TaskId,
    pub generation: u64,
}
```

Every long-running effect should carry the task generation that spawned it. When the result returns, domain checks whether that generation is still current.

Rules:

```text
- canceled task generation rejects late harness results;
- superseded retrieval rejects late search/vector/web results;
- changed source artifact rejects late parser/indexer output for old content hash;
- policy profile change can invalidate queued risky effects.
```

### 9.4 Watchdog and Transition Journal

Brioche's `EngineWatchdog` and `TransitionJournal` should become Maestria runtime components.

Maestria should record before executing each domain transition:

```text
domain input
current task id if any
generation id
source effect id if any
started timestamp at runtime edge
transition result summary
emitted events/effects
```

If the runtime crashes, Maestria can replay uncommitted transitions or mark them uncertain.

### 9.5 Harness Adapter Contract

The previous report's harness contract should be strengthened.

```rust
pub struct HarnessRequest {
    pub run_id: HarnessRunId,
    pub task_id: Option<TaskId>,
    pub generation: Option<u64>,
    pub capability: HarnessCapability,
    pub scope: ScopeId,
    pub risk: RiskClass,
    pub approval: Option<ApprovalId>,
    pub payload: HarnessPayload,
}
```

```rust
pub struct HarnessOutcome {
    pub run_id: HarnessRunId,
    pub status: HarnessStatus,
    pub stdout: Option<BlobId>,
    pub stderr: Option<BlobId>,
    pub structured: Option<HarnessStructuredOutput>,
    pub artifacts: Vec<ArtifactId>,
    pub diff: Option<BlobId>,
    pub started_at: RuntimeTimestamp,
    pub finished_at: RuntimeTimestamp,
}
```

The harness adapter should never return unstructured truth. It returns an outcome that Maestria turns into evidence.

---

## 10. Storage Architecture Updates

The previous report's storage defaults remain good, but Brioche adds hot/cold separation and persistence discipline.

### 10.1 Current State vs Event History vs Blobs vs Projections

```text
SQLite relational tables = current queryable state
append-only domain_events = audit/replay/debug trail
filesystem blobs = immutable large content
Tantivy = full-text projection
sqlite-vec = semantic projection later
SQLite edge tables = graph projection later
```

Rule:

```text
State is queryable.
Events are accountable.
Evidence is immutable.
Projections are rebuildable.
```

### 10.2 Hot/Cold State Split

Use Brioche's hot/cold idea for Maestria tasks and ingestion.

Hot state:

```text
small task state
generation ids
current artifact processing status
policy decisions
validation status
small metadata
```

Cold state:

```text
large extracted text
command logs
web snapshots
PDF text
full evidence packs
large generated reports
embeddings
screenshots
```

Cold state lives in blobs/projections. Hot state points to it by ID/hash.

### 10.3 Redb vs SQLite Decision

Brioche uses Redb for shell persistence. The existing Maestria report recommends SQLite + SQLx. Keep SQLite as Maestria's default because Maestria needs queryable metadata, joins, inspection, and user-facing knowledge records. Borrow Brioche's persistence discipline, not necessarily its database.

Recommended:

```text
MVP metadata/event log: SQLite + SQLx
MVP blobs: filesystem content-addressed store
MVP search: Tantivy
Later vector: sqlite-vec
Later graph: SQLite edge tables
```

---

## 11. Policy and Governance Updates

The previous report says policy must be a subsystem. Brioche shows policy should be capability traits with fixed injection points.

### 11.1 Governance Hooks

Maestria should define policy hook points around risky operations:

```text
before_effect_execution
before_harness_request
before_memory_promotion
before_task_completion
before_workspace_write
before_web_fetch
before_index_secret_candidate
after_validation_report
after_contradiction_detected
```

### 11.2 Governance Traits

```rust
pub trait ScopeGuard {
    fn check_scope(&self, effect: &MaestriaEffect, scope: &Scope) -> PolicyDecision;
}

pub trait ApprovalGate {
    fn approval_for(&self, decision: PolicyDecision, profile: AutonomyProfile) -> ApprovalRequirement;
}

pub trait ValidationGate {
    fn completion_status(&self, task: &Task, report: &ValidationReport) -> CompletionStatus;
}

pub trait MemoryPromotionGate {
    fn decide(&self, candidate: &MemoryCandidate, evidence: &[Evidence]) -> MemoryPromotionDecision;
}
```

### 11.3 Policy Profiles

Like Brioche governance profiles, Maestria should ship policy profiles:

```text
ReadOnly
Assisted
ScopedAutonomy
StrictResearch
TrustedWorkspace
```

Profiles configure gates. They do not modify domain mechanism types.

---

## 12. Validation as a First-Class Gate

The existing report already emphasizes validation. Brioche's test and invariant culture suggests making validation a domain transition requirement.

### 12.1 Completion Status

```rust
pub enum CompletionStatus {
    CompletedVerified { report: ValidationReportId },
    CompletedWithWarnings { report: ValidationReportId, warnings: Vec<ValidationWarning> },
    Blocked { reason: BlockingReason },
    Failed { reason: FailureReason, evidence: Vec<EvidenceId> },
}
```

There should be no generic `Completed` state.

### 12.2 Validation Report Shape

```text
task id
claim checked
method used
commands/tests run if any
evidence ids
harness run ids
pass/fail/warn status
known gaps
freshness status
policy status
```

### 12.3 Validator Types

Initial validators:

```text
CitationValidator
EvidenceExistenceValidator
SourceFreshnessValidator
TaskStateValidator
HarnessRunValidator
CommandExitValidator
DiffSummaryValidator
SecretLeakValidator
PromptInjectionBoundaryValidator
RustProjectValidator
```

For Maestria's own repository, the standard validation profile should include:

```text
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace --doc --all-features
cargo deny check all
custom philosophy-check.py
contract tests for touched adapters
```

---

## 13. Verification and CI Architecture

Brioche's verification stack is the strongest repository-level improvement.

### 13.1 Required CI Gates

```text
Gate 0 — static hygiene
  cargo fmt --check
  nightly rustfmt if using import grouping
  cargo deny check all
  cargo machete
  branch/commit convention check if desired

Gate 1 — doctrine and invariants
  cargo maestria-lint
  invariant reference checker
  philosophy-check.py
  RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps

Gate 2 — tests
  cargo test --workspace --lib --bins --tests --all-features
  cargo test --workspace --doc --all-features
  property tests for state machines
  contract tests for adapters
  parser golden tests

Gate 3 — profiles and benchmarks
  cargo check --workspace --profile headless
  cargo check --workspace --profile daemon
  cargo check --workspace --profile desktop if desktop exists
  cargo check --workspace --benches
  smoke-run critical benchmarks

Gate 4 — security
  cargo audit / RustSec
  secret fixture tests
  prompt-injection boundary tests
```

### 13.2 Philosophy Checker

Maestria should create `scripts/philosophy-check.py` early.

Initial checks:

```text
- no unwrap/expect/panic in crates/kernel/maestria-domain
- no tokio/sqlx/reqwest/tantivy imports in maestria-domain
- no bare TODO/FIXME
- public hot-path items mention invariant refs
- public async fn in runtime documents cancel safety
- internal mpsc channels document capacity/drop policy
- no stale .orig/.rej/.bak/.tmp/.swp files in source/workflow paths
- no adapter-specific request/result types in domain or ports
- no direct SQL mutation in CLI/API handlers
- no generated blobs under production source paths
```

### 13.3 Contract Tests

Every adapter must pass shared contracts:

```text
ArtifactRepositoryContract
EventLogContract
BlobStoreContract
FullTextIndexContract
VectorIndexContract
GraphStoreContract
ParserContract
HarnessAdapterContract
WebEvidenceProviderContract
ValidationContract
```

Example contract requirement:

```text
BlobStoreContract:
- storing same bytes returns same content hash;
- loading by hash returns exact bytes;
- missing hash returns typed NotFound;
- writes are atomic or leave no visible partial blob;
- path traversal cannot escape blob root.
```

### 13.4 Property Tests

Use proptest for:

```text
task state transitions
memory candidate lifecycle
evidence span ranges
content hash idempotency
policy decision composition
event replay equivalence
parser chunk boundary invariants
retrieval score fusion monotonicity properties where applicable
```

### 13.5 Replay Tests

Borrow Brioche's replay/determinism idea.

```text
Given the same ordered DomainInput stream and initial state,
Maestria domain replay must produce the same DomainEvent stream and final state.
```

Runtime timestamps and external outputs are inputs to replay, not sampled during replay.

---

## 14. Updated MVP Build Order

The previous report's MVP order is good but should move doctrine and enforcement earlier.

### 14.1 Phase 0 — Repository Immune System

Build first:

```text
1. Cargo workspace skeleton
2. docs/PHILOSOPHY.md
3. docs/SPECS.md initial invariant ledger
4. rustfmt.toml, clippy.toml, deny.toml
5. .github/PULL_REQUEST_TEMPLATE.md with invariant checklist
6. scripts/philosophy-check.py with initial lightweight checks
7. CI with fmt, clippy, docs, tests, deny, philosophy-check
```

Done when:

```text
A PR can fail because it violates architecture doctrine before feature code exists.
```

### 14.2 Phase 1 — Domain Kernel

```text
maestria-domain
- strong IDs
- Artifact, Evidence, Claim, Relation
- DomainEvent
- DomainInput
- MaestriaEffect
- Task state machine
- MemoryCandidate lifecycle
- Validation status types
- typed DomainError
- replay tests
- property tests for transitions
```

### 14.3 Phase 2 — Ports and Contracts

```text
maestria-governance
maestria-ports or equivalent contract module
- ArtifactRepository
- EventLog
- BlobStore
- FullTextIndex
- Parser
- HarnessAdapter
- ValidationRunner
- Policy traits
- contract test harnesses
```

### 14.4 Phase 3 — Local Storage Backbone

```text
maestria-storage-sqlite
maestria-blob-fs
- current state tables
- append-only events
- content-addressed blobs
- migrations
- idempotency tests
- crash/partial-write tests
```

### 14.5 Phase 4 — Parsing and Full-Text Search

```text
maestria-parsers
maestria-search-tantivy
- Markdown/plain text/Rust/Cargo parser basics
- structure-aware chunks
- Tantivy index projection
- source spans
- parser golden tests
```

### 14.6 Phase 5 — Core Services and Runtime

```text
maestria-runtime
service orchestration
- effect executor
- bounded queues
- cancellation registry
- ingestion worker
- task transition worker
- transition journal
- watchdog
- graceful shutdown
```

### 14.7 Phase 6 — CLI and Daemon

```text
maestria-cli
maestria-daemon
- init
- index
- search
- open-evidence
- task start/show
- memory candidates
- doctor
```

### 14.8 Phase 7 — Harness Gateway

```text
maestria-harness
maestria-harness-cli
- capability declaration
- scope/risk classification
- run logs
- output blobs
- approval bridge
- HarnessAdapterContract
```

### 14.9 Phase 8 — Validation and Memory Promotion

```text
maestria-validation
maestria-memory
- validation reports
- completion gates
- memory candidate review
- promotion policy
- contradiction/staleness markers
```

Only after these should Maestria add vector search, graph expansion, web evidence, browser harnesses, desktop UI, and advanced code intelligence.

---

## 15. Updated MVP Definition of Done

MVP is done when all are true:

```text
- Repository has enforceable PHILOSOPHY.md and SPECS.md.
- CI runs fmt, clippy, docs, tests, deny/audit, and philosophy checks.
- Domain kernel has no I/O dependencies and no panic/unwrap/expect path.
- Domain replay of an input stream is deterministic.
- Task transitions are typed and property-tested.
- Events are append-only and persisted.
- Evidence objects cite immutable blobs or source spans.
- Approved folders can be indexed idempotently.
- Markdown/plain text/Rust/Cargo metadata can be parsed and searched.
- Search results can be opened as evidence.
- Memory candidates can be created but not silently promoted.
- Task folders can be created and linked to evidence.
- User files are not modified without approval.
- CLI/API/daemon call services; they do not mutate stores directly.
- At least one storage adapter passes contract tests.
- At least one parser has golden tests.
- Harness adapter trait exists and a minimal CLI harness passes its contract.
- Validation report type exists and gates task completion.
```

---

## 16. Architecture Traps to Avoid

### 16.1 Do Not Let Retrieval Become the Architecture

Vector search, Tantivy, and graph edges are projections. The architecture is:

```text
Evidence + Events + Domain State + Policy + Validation
```

Retrieval serves that architecture.

### 16.2 Do Not Let the Harness Own Truth

A harness result is evidence, not truth.

```text
Harness says command passed.
Maestria stores command output as evidence.
Validation decides whether the task can be trusted.
```

### 16.3 Do Not Put Policy in Domain Mechanism

Bad:

```text
Task transition function checks user tier, approval UI mode, provider type.
```

Good:

```text
Domain transition requests an effect.
Governance classifies risk and approval requirement.
Runtime executes only approved effects.
```

### 16.4 Do Not Over-Split Crates

Brioche's philosophy explicitly rejects one-file-per-type cargo culting. Maestria should not create 30 crates before boundaries are real.

Start with fewer crates grouped by book. Split when:

```text
- dependency boundary requires it;
- contract tests need independent adapter implementation;
- compile surface becomes too large;
- invariant owner becomes independent.
```

### 16.5 Do Not Use `serde_json::Value` as a Domain Escape Hatch

Raw JSON is acceptable at adapter edges. It should not cross into core domain when the shape is known.

### 16.6 Do Not Treat Documentation as Optional

In Maestria, architecture docs are not decoration. They are a compatibility surface between code, tests, review, and future agents.

---

## 17. Immediate Implementation Recommendations

Because `/home/arobin/Dev/maestria` currently only shows `LICENSE`, the cleanest next steps are:

1. Create the Rust workspace skeleton.
2. Add `docs/PHILOSOPHY.md` before feature code.
3. Add `docs/SPECS.md` with the initial invariant ledger.
4. Add CI and local scripts that enforce the first rules.
5. Implement `maestria-domain` with task/evidence/event/effect types.
6. Add replay and property tests before adapters.
7. Add SQLite/blob adapters and their contract tests.
8. Add parser/search MVP.
9. Add runtime effect executor and bounded queues.
10. Add minimal harness adapter only after policy/evidence/event flow exists.

This differs from the old MVP order by moving repository doctrine and deterministic kernel proof to the front.

---

## 18. Final Architecture Definition

Maestria is:

> **A Rust-native, local-first second-brain operating layer for AI agents. It owns durable memory, evidence, task state, validation, policy, source-grounded retrieval, and workspace organization. It connects to the user's machine and internet through swappable harnesses, but every side effect is policy-gated, event-recorded, evidence-backed, and validation-checked.**

The Brioche-informed engineering sentence is:

```text
Maestria is a modular monolith with an enforceable philosophy, a pure deterministic domain kernel, explicit declarative effects, injected governance policies, local-first storage, rebuildable projections, bounded async runtime bridges, and contract-tested adapters.
```

The short product sentence remains:

```text
Maestria is the AI's second brain; the harness is its hands, eyes, terminal, browser, and connection to the machine.
```

The new repository rule is:

```text
If the philosophy cannot be enforced, it is not architecture yet.
```

---

## 19. Evidence Used for This Revision

Observed source report:

- `/home/arobin/Téléchargements/maestria_updated_code_architecture_report(3).md`

Observed Maestria repository state:

- `/home/arobin/Dev/maestria` contained `LICENSE` only in the visible root listing.

Observed Brioche documentation and governance:

- `/home/arobin/Dev/Brioche/docs/PHILOSOPHY.md`
- `/home/arobin/Dev/Brioche/docs/SPECS.md`
- `/home/arobin/Dev/Brioche/.github/workflows/ci.yml`
- `/home/arobin/Dev/Brioche/.github/PULL_REQUEST_TEMPLATE.md`
- `/home/arobin/Dev/Brioche/scripts/philosophy-check.py`
- `/home/arobin/Dev/Brioche/Cargo.toml`

Observed Brioche implementation patterns:

- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/lib.rs`
- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/engine.rs`
- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/plugin.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-runtime/src/lib.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-runtime/src/shell.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-runtime/src/effect_executor.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-runtime/src/unified_event_bus.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-runtime/src/engine_watchdog.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-persistence/src/lib.rs`
- `/home/arobin/Dev/Brioche/crates/providers/brioche-provider-openai/src/client/request_flow.rs`
- `/home/arobin/Dev/Brioche/crates/tools/brioche-tools-system/src/registry.rs`

---

## 20. Additional Brioche Findings Incorporated from Parallel Scouts

The parallel repository scouts confirmed the same architectural direction and added four concrete refinements that should be treated as required, not optional.

### 20.1 Type-State Builders for Mandatory Wiring

Brioche's `BriocheEngineBuilder<DA, LG>` uses `Missing` / `Present` type-state markers to enforce mandatory governance wiring before an engine can be built. Maestria should copy this pattern for components that are not optional in production:

```rust
pub struct MaestriaKernelBuilder<P, V, E> {
    policy: P,
    validation: V,
    event_log: E,
}
```

Required production wiring:

```text
- Policy engine present
- Validation gate present
- Event log present
- Blob store present
- At least one evidence repository present
- Runtime effect executor present
```

Development profiles may provide explicit no-op implementations, but the no-op must be visible in the type or profile name. Silent omission is not acceptable.

### 20.2 ADRs Need Affected Invariants, Not Only Context

Brioche's ADR template requires status, context, decision, consequences, affected invariants, and book references. Maestria should adopt that exact discipline.

ADR trigger rules:

```text
- crossing a book/layer boundary;
- changing domain event shape;
- changing task/memory/evidence lifecycle;
- adding a new storage/projection backend;
- changing harness capability semantics;
- weakening or adding autonomy policy;
- adding a new source of nondeterminism;
- changing validation completion rules.
```

Every ADR must include:

```text
Status:
Context:
Decision:
Consequences:
Affected invariants:
Affected crates/books:
Rejected alternatives:
Verification changes:
```

### 20.3 CODEOWNERS and Review Boundaries Are Architecture

Brioche uses CODEOWNERS by architecture surface, with stricter ownership for core, macro, property, replay, governance, docs/specs, CI, and tooling.

Maestria should define CODEOWNERS around invariant ownership:

```text
docs/PHILOSOPHY.md                      @architecture-owners
docs/SPECS.md                           @architecture-owners
docs/adr/**                             @architecture-owners
crates/kernel/maestria-domain/**        @domain-owners
crates/kernel/maestria-governance/**    @policy-owners
crates/runtime/**                       @runtime-owners
crates/storage/**                       @storage-owners
crates/harness/**                       @harness-owners @security-owners
tests/property/**                       @domain-owners
tests/replay/**                         @domain-owners @runtime-owners
.github/workflows/**                    @infra-owners
scripts/philosophy-check.py             @architecture-owners @infra-owners
```

This prevents future agents or contributors from changing the kernel, harness, policy, or invariant documents without the right review context.

### 20.4 Dependency and Source Governance Belong in the Architecture Report

Brioche's `deny.toml`, `.cargo/audit.toml`, cargo-deny gate, cargo-machete gate, duplicate dependency checks, and explicit registry/source policy show that dependency governance is an architectural boundary.

Maestria should require:

```text
- narrow allowed license set;
- denied unknown registries;
- denied unknown git sources;
- advisory ignores with owner and review date;
- unused dependency detection;
- duplicate dependency reporting;
- explicit rule that domain/kernel crates cannot add heavyweight adapter dependencies;
- PR checklist item for any new dependency explaining layer, invariant, and alternative considered.
```

### 20.5 Runtime Persistence Should Use DTO Boundaries

Brioche persistence separates in-memory session structures from persisted DTOs and uses append-only message deltas plus cold blob storage. Maestria should use the same DTO rule:

```text
Domain type != database row != API response != harness payload.
```

DTOs are allowed at boundaries. Domain types remain the source of invariant meaning.

Recommended DTO boundaries:

```text
ArtifactRow             -> Artifact
EvidenceRow             -> Evidence
DomainEventRow          -> DomainEvent
TaskRow + TaskEventRows -> Task
HarnessRunRow           -> HarnessOutcome summary
BlobManifestRow         -> BlobRef
ApiTaskResponse         <- Task + ValidationReport summary
HarnessPayloadDTO       <- HarnessRequest
```

### 20.6 Release Verification Should Be Stricter Than PR Verification

Brioche release workflow repeats format, clippy, tests, docs, dependency checks, benchmarks, signed tag verification, and locked profile builds. Maestria should keep PR gates fast but make release gates exhaustive:

```text
PR:
  fmt, clippy, docs, tests, deny, philosophy-check, touched contract tests

main:
  full workspace tests, replay/property tests, profile checks, audit, benchmark smoke

release:
  locked dependency build, full benchmarks, migration checks, replay corpus,
  signed tag verification, packaged CLI/daemon smoke test
```

### 20.7 Updated Evidence Additions

Additional Brioche files verified by the scouts:

- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/engine/builder.rs`
- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/engine/router.rs`
- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/engine/hooks.rs`
- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/engine/dispatch.rs`
- `/home/arobin/Dev/Brioche/crates/kernel/brioche-core/src/engine/transition_support.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-runtime/src/transition_journal.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-persistence/src/storage/redb.rs`
- `/home/arobin/Dev/Brioche/crates/runtime/brioche-shell-persistence/src/storage/cache.rs`
- `/home/arobin/Dev/Brioche/docs/architecture/book-i-core.md`
- `/home/arobin/Dev/Brioche/docs/architecture/book-ii-governance.md`
- `/home/arobin/Dev/Brioche/docs/architecture/book-iiic-shell-projection.md`
- `/home/arobin/Dev/Brioche/docs/adr/README.md`
- `/home/arobin/Dev/Brioche/CONTRIBUTING.md`
- `/home/arobin/Dev/Brioche/docs/first-pr-guide.md`
- `/home/arobin/Dev/Brioche/.github/CODEOWNERS`
- `/home/arobin/Dev/Brioche/deny.toml`
- `/home/arobin/Dev/Brioche/.cargo/audit.toml`
- `/home/arobin/Dev/Brioche/.github/workflows/benchmarks.yml`
- `/home/arobin/Dev/Brioche/.github/workflows/release.yml`
