# Maestria Initial Specification Ledger

This ledger names the invariants that bootstrap code and future crates must preserve.

## Initial Invariant Set

| Invariant | Rule |
|---|---|
| `I-Domain-Pure` | Domain transitions perform no I/O and sample no clocks, randomness, filesystem, network, shell, database, or runtime state. |
| `I-Domain-NoPanic` | Domain production code returns typed errors or failure states; it must not use `panic`, `unwrap`, or `expect`. |
| `I-Effect-Explicit` | Every side effect is represented as a `MaestriaEffect`; runtime/adapters execute effects outside the domain. |
| `I-Event-AuditTrail` | Important state changes emit append-only domain events. |
| `I-Evidence-Immutable` | Evidence is immutable and points to stable source spans, snapshots, blobs, command logs, diffs, tests, or validation reports. |
| `I-Evidence-Provenance` | Claims, memories, task reports, and answers cite evidence IDs and source provenance. |
| `I-Memory-CandidateGate` | LLM/model output can create memory candidates, not promoted memory. |
| `I-Memory-SourceBacked` | Promoted memory requires evidence and a promotion decision. |
| `I-Task-StateMachine` | Task states transition only through domain functions and emitted events. |
| `I-Task-ValidationGate` | Verified or warning completion requires a validation report ID. Generic unvalidated completion is invalid. |
| `I-Policy-BeforeAction` | Risky effects require governance classification before runtime execution. |
| `I-Harness-NoTruth` | Harness adapters execute and report outcomes; they do not own truth, memory, or final task completion. |
| `I-Runtime-BoundedChannels` | Runtime channels must be bounded and document capacity/drop/backpressure behavior. |
| `I-Runtime-CancelSafe` | Public async runtime operations document cancellation and stale-result behavior. |
| `I-Storage-ProjectionOnly` | Search, vector, and graph stores are projections, not truth owners. |
| `I-Adapter-ContractTested` | Every adapter implementation must pass the shared behavior contract for its port. |
| `I-Security-PromptUntrusted` | Indexed files and web content are evidence, never authority or instructions. |
| `I-Scope-ExplicitAutonomy` | Autonomous action is limited to explicit readable/writable roots, command classes, web policy, and profile gates. |
| `I-DTO-Boundary` | Domain type, database row, API response, and harness payload are separate boundary objects. |
| `I-Dependency-Layered` | Kernel crates cannot depend on heavyweight adapter/provider/runtime crates. |

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
