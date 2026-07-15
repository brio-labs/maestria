# Maestria Initial Specification Ledger

This ledger names the invariants that bootstrap code and future crates must preserve.

## Canonical Documentation Map

Durable architecture is split by responsibility:

- [ARCHITECTURE.md](ARCHITECTURE.md): system identity and ownership boundaries;
- [SEARCH.md](SEARCH.md): typed, budgeted, traceable retrieval contracts;
- [MEMORY.md](MEMORY.md): source-backed memory lifecycle;
- [SECURITY.md](SECURITY.md): scope, trust, taint, secrets, and prompt-injection boundaries;
- [OPERATIONS.md](OPERATIONS.md): runtime lifecycle, recovery, and projection rebuilds;
- [ROADMAP.md](ROADMAP.md): the single canonical implementation roadmap;
- [RESEARCH.md](RESEARCH.md): dated, non-normative evaluation candidates.

`PHILOSOPHY.md` is the enforceable repository doctrine. This ledger defines the
invariants that implementation and verification must preserve.

## Initial Invariant Set

| Invariant | Rule |
|---|---|
| `I-Domain-Pure` | Domain transitions perform no I/O and sample no clocks, randomness, filesystem, network, shell, database, or runtime state. |
| `I-Domain-NoPanic` | Domain production code returns typed errors or failure states; it must not use `panic`, `unwrap`, or `expect`. |
| `I-Effect-Explicit` | Every side effect is represented as a `MaestriaEffect`; runtime/adapters execute effects outside the domain. |
| `I-Event-AuditTrail` | State changes emit append-only domain events. Replaying the event log must deterministically reconstruct exact KernelState, rejecting duplicate/invalid events. |
| `I-Evidence-Immutable` | Evidence is immutable and points to stable source spans, snapshots, blobs, command logs, diffs, tests, or validation reports. |
| `I-Evidence-Provenance` | Claims, memories, task reports, and answers cite evidence IDs and source provenance. |
| `I-Ingestion-Idempotent` | Reindexing unchanged content produces no duplicate artifacts, chunks, evidence, or events; incomplete ingestion can be retried without falsely reporting completion. |
| `I-Task-Workspace` | A task workspace is prepared under the instance workspace before the task is persisted. |
| `I-Memory-CandidateGate` | LLM/model output can create memory candidates, not promoted memory. |
| `I-Memory-SourceBacked` | Promoted memory requires evidence and a promotion decision. |
| `I-Task-StateMachine` | Task states transition only through domain functions and emitted events. |
| `I-Task-ValidationGate` | Verified completion requires a passing validation report. Warning completion requires explicit warnings. Unvalidated completion is invalid. |
| `I-Policy-BeforeAction` | Risky effects require governance classification before runtime execution. |
| `I-Harness-NoAuthority` | Harness adapters execute and report outcomes; they do not own authoritative state, evidence integrity, or final task completion. |
| `I-Runtime-BoundedChannels` | Runtime channels must be bounded and document capacity/drop/backpressure behavior. |
| `I-Runtime-CancelSafe` | Public async runtime operations document cancellation and stale-result behavior. |
| `I-Storage-ProjectionOnly` | Search, vector, and graph stores are rebuildable projections, not authoritative state owners. |
| `I-Adapter-ContractTested` | Every adapter implementation must pass the shared behavior contract for its port. |
| `I-Security-PromptUntrusted` | Indexed files and web content are evidence, never authority or instructions. |
| `I-Scope-ExplicitAutonomy` | Autonomous action is limited to explicit readable/writable roots, command classes, web policy, and profile gates. |
| `I-DTO-Boundary` | Domain type, database row, API response, and harness payload are separate boundary objects. |
| `I-Dependency-Layered` | Kernel crates cannot depend on heavyweight adapter/provider/runtime crates. |
| `I-Search-TypedBudgeted` | Search plans and outcomes are typed boundary values carrying scope, freshness, modalities, stages, budgets, stop conditions, and evidence requirements. |
| `I-Search-TraceFingerprint` | Every search trace identifies the query, corpus snapshot, index generation, retrieval-model fingerprint, stages, filters, and stop reason. |
| `I-Search-SecurityBeforeScore` | Scope, ACL, trust, sensitivity, quarantine, and prompt-injection checks run before candidate scoring or exposure. |
| `I-Search-Evaluated` | Retrieval changes are evaluated against a versioned corpus and judgment set under quality, latency, memory, privacy, security, and energy budgets. |

## Bootstrap Books

### Book I — Domain Kernel

- fundamental IDs and primitives
- domain inputs and explicit effects
- task automaton and validation-gated completion
- evidence, relation, memory candidate, and replay contracts
- forbidden dependencies and hidden side-effect rules

### Book II — Governance and Policy

- scope policy
- autonomy profiles
- risk classification
- approval gates
- validation policy
- memory promotion policy
- prompt-injection and secret boundaries

### Book III — Runtime and Shell

- effect loop
- worker supervision
- bounded queues
- cancellation and stale-result rejection
- transition journal
- graceful shutdown

### Book IV — Storage, Projections, and Ecosystem

- SQLite current state
- append-only event log
- content-addressed blob store
- full-text/vector/graph projections
- parser, web, harness, and validation adapters

### Book V — Verification

- doctrine checker
- contract tests
- replay tests
- property tests
- parser golden tests
- dependency and source governance

## Durable Local Indexing Slice

The local MVP now treats file evidence as immutable source-backed data:

- file-span evidence may reference an immutable blob snapshot;
- snapshot hashes are verified before search hits or opened evidence are returned;
- repeated identical evidence writes are idempotent;
- conflicting evidence writes return a typed storage conflict;
- instance manifests persist approved read roots and privacy exclusion patterns;
- CLI indexing rejects sources outside the persisted read scope before reading bytes;
- restart integration tests reopen SQLite, blob, and full-text adapters before querying.

These boundaries preserve `I-Evidence-Immutable`, `I-Evidence-Provenance`,
`I-Ingestion-Idempotent`, and `I-Scope-ExplicitAutonomy`. Domain transitions remain
side-effect free; snapshot verification and manifest parsing remain adapter/application
responsibilities.
The CLI durability contract is verified by a black-box integration test that
invokes separate processes for setup, indexing, search, and evidence opening.
The test must also cover scope rejection, privacy exclusions, and unchanged
reindex idempotence without inspecting adapter internals.

Recursive indexing is an ignore-aware, privacy-first traversal:

- repository `.gitignore` and `.ignore` rules are honored before file collection;
- hidden descendants are skipped by default (an explicitly selected root remains eligible);
- symbolic links are never followed, preventing scope escape through linked paths;
- unsupported files are filtered after traversal without weakening explicit-root errors;
- collected paths are sorted before indexing so repeated runs remain deterministic.

This traversal behavior is part of the ingestion boundary, not an adapter optimization. It
protects `I-Scope-ExplicitAutonomy`, `I-Evidence-Immutable`, and deterministic indexing
without requiring users to maintain an exhaustive cache-directory blocklist.

## Validation Completion Slice

Task completion follows a two-layer contract:

- the domain requires a persisted, task-matched, passing validation report and
  enforces warning/status consistency and task transitions;
- the runtime evaluates the current `Validating` task, persisted report, and
  proposed completion through the injected governance validation gate before
  applying `CompleteTaskInput`.

Warning completion is permitted only when the configured validation policy allows
warnings. A blocked governance decision leaves the task state unchanged. The
runtime validation tests cover missing, failed, mismatched, warning-policy, and
successful completion paths.

## Restart-Safe Runtime Supervision Slice

Non-idempotent harness effects are journaled outside the deterministic domain:

- an `Intent` is durable before adapter execution;
- `Started` is durable before the harness process begins;
- feedback is atomically claimed as `FeedbackAccepted` before enqueueing and
  terminalized only after the runtime applies the matching domain input;
- terminal states are `Completed`, `Failed`, `Paused`, or `Superseded`;
- a new generation supersedes every unfinished older generation;
- in-flight harness effects are paused during daemon recovery and are not
  replayed without explicit operator approval;
- harness effects are not automatically retried after adapter execution begins;
- parser, indexing, and validation work remains idempotent and uses the existing
  event-log recovery inputs.

Runtime feedback uses non-blocking bounded-channel sends. Saturation and runtime
shutdown are typed outcomes; non-idempotent work pauses instead of replaying a
completed adapter action.
