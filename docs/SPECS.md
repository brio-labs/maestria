# Sillage Initial Specification Ledger

This ledger names the invariants that bootstrap code and future crates must preserve.

## Canonical Documentation Map

Durable architecture is split by responsibility:

- [ARCHITECTURE.md](ARCHITECTURE.md): system identity and ownership boundaries;
- [SEARCH.md](SEARCH.md): typed, budgeted, traceable retrieval contracts;
- [MEMORY.md](MEMORY.md): source-backed memory lifecycle;
- [SECURITY.md](SECURITY.md): scope, trust, taint, secrets, and prompt-injection boundaries;
- [OPERATIONS.md](OPERATIONS.md): runtime lifecycle, recovery, and projection rebuilds;
- [ROADMAP.md](ROADMAP.md): the single canonical product roadmap;
- [RESEARCH.md](RESEARCH.md): dated, non-normative evaluation candidates;
- [ORCHESTRATION.md](ORCHESTRATION.md): execution ownership, assignment, evidence, and stop/closure procedure; no product priority;
- [ADR-0011](adr/ADR-0011-source-retention-public-api.md): adopted source-retention and public-version doctrine; implementation pending.

`PHILOSOPHY.md` is the enforceable repository doctrine. This ledger defines the
invariants that implementation and verification must preserve.

## Initial Invariant Set

| Invariant | Rule |
|---|---|
| `I-Domain-Pure` | Domain transitions perform no I/O and sample no clocks, randomness, filesystem, network, shell, database, or runtime state. |
| `I-Domain-NoPanic` | Domain production code returns typed errors or failure states; it must not use `panic`, `unwrap`, or `expect`. |
| `I-Domain-ValidStates` | Domain values make known invalid state combinations unrepresentable: exclusive states carry their payloads in enum variants, validated values and meaningful identities have distinct types, boundary conversion owns runtime validation, and state-dependent operations use exhaustive typed transitions or justified typestate. |
| `I-Effect-Explicit` | Every side effect is represented as a `SillageEffect`; runtime/adapters execute effects outside the domain. |
| `I-Event-AuditTrail` | Important domain state changes emit append-only events and replay reconstructs exact KernelState. This does not require a durable event for each search or a persistent query history; search traces follow source-retention policy. |
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
| `I-Search-TraceFingerprint` | A runtime trace identifies query and retrieval context (corpus snapshot, index generation, model fingerprint, stages, filters, budgets, stop reason) during its authorized lifetime; traces are ephemeral by default and do not persist query/path/content/catalogue data in a no-document-persistence scope. |
| `I-Search-SecurityBeforeScore` | Scope, ACL, trust, sensitivity, quarantine, and prompt-injection checks run before candidate scoring or exposure. |
| `I-Search-Evaluated` | Retrieval changes are evaluated against a versioned corpus and judgment set under quality, latency, memory, privacy, security, and energy budgets. |
| `I-Source-PolicyAxes` | Scope, execution place, timing, documentary retention, representation, and budgets are independent per-source choices; preferences and grants are control state, and local consent never implies remote authorization. |
| `I-Source-NoDocumentPersistence` | When selected, no Sillage-controlled store persists the source's query, document paths, discovered-file catalogue, content, derivatives, or query-linked trace. An approved-root preference or grant may persist only as separately chosen control state; OS and external-service limits are named. |
| `I-Source-Retirement` | Pause, disable, or retirement does not mean purge. Purge is separate, explicit, and governed; where required, removal and its verification consequence are explicit without silently rewriting append-only history. |
| `I-API-VersionTransition` | Public protocol negotiation is intentional and versioned; a dated bounded transition requires a separately approved ADR and removal criterion. Internal changes are clean cutovers; permanent aliases and shims are prohibited. |
| `I-Platform-Qualification` | Linux, Windows, and macOS are mandatory product targets, but installed qualification is OS/configuration-specific; compilation or proof on another OS does not qualify a platform. |

## Planned Product Contract

This subsection defines the target product contract, not a claim that every
launcher or extension capability ships. Sillage's core is a native retrieval
launcher and reusable search boundary; generation and chat are optional
clients/plugins. Local use does not require an account, model, or pre-existing
document index. Declining preparation remains a useful bounded retrieval path,
not an exhaustive instant semantic scan of the computer.

Linux, Windows, and macOS are all mandated. Qualification is distinct from
that target: existing Linux evidence is bounded to its named configurations
and tasks; Windows and macOS remain unqualified. This section is the canonical
objective/profile ledger ([`SPECS.md#mandated-product-objectives`](SPECS.md#mandated-product-objectives)).
[`ORCHESTRATION.md`](ORCHESTRATION.md) is the execution-only ticket crosswalk;
it does not define acceptance or completion profiles. The launcher and
process boundaries are defined in
[`ARCHITECTURE.md`](ARCHITECTURE.md#target-product-architecture), and the
planned extension threat model is in
[`SECURITY.md`](SECURITY.md#planned-extension-threat-model).

Source policies keep scope, execution place, computation timing, documentary
retention, representation, and resource budgets independent per source. A
source choice is not one indivisible profile: a local source may be searched
on demand without a durable index, and representation choice does not silently
change source scope. Approved-root preferences and provider/consumer grants are
control state, distinct from document content; their persistence requires a
separate explicit choice, and session-only control is available. Local approval
does not authorize remote disclosure.

When a no-document-persistence choice applies, Sillage-controlled stores must
not retain that source's query, document paths, discovered-file catalogue,
content, derivatives, temporary copies, caches, or query-linked traces.
Approved-root or grant control state may persist only when its separate
retention choice allows it. Traces are ephemeral by default. Ordinary
telemetry export is opt-in; evidence for a voluntary experiment requires
separate consent and purpose. OS caches, swap, external-provider logs, and
backups outside Sillage's control are named limitations, not permission to
leave data in controlled stores.

Pause, disable, retirement, and purge are distinct. Retirement stops serving a
representation but does not erase it. Purge is an explicit, separately
authorized retention action; where deletion is required, removal and its
verification consequence are explicit without silently rewriting append-only
domain events. A refusal to index or persist does not promise exhaustive
coverage: report search scope, depth, stop reason, and partial coverage so that
no result is not misrepresented as proof that nothing exists.

The canonical product milestones and current exit criteria/status are in
[`ROADMAP.md`](ROADMAP.md), the sole product-priority authority; this
specification does not maintain a second schedule.

The current Linux source includes extension SDK/callbacks, brokered capability
dispatch, installed-bundle loading, and sandboxed-worker execution paths.
Complete target-contract details and adversarial security, native/OS, and
installed-product qualification remain pending. These source paths are evidence
of implementation, not proof of target-contract compliance or release
qualification. This specification does not claim all extension invariants are
mechanically enforced, publish SDK exports, or invent exact API/wire schemas.

## Mandated Product Objectives

This is the canonical acceptance matrix for the 18 objectives in the adopted
mandate. It records required outcomes and evidence, not implementation status.
The static audit and kit do not qualify these objectives; current delivery
status belongs to [`ROADMAP.md`](ROADMAP.md) and linked GitHub issues. An
intermediate lexical release must satisfy all criteria applicable to its
shipped OS and path, including lexical relevance, rights, budgets, packaging,
and UX. It does not close the full product/mandate and requires no neural
promotion.

| ID | Required outcome and evidence | Completion profile |
|---|---|---|
| OBJ-01 | Open-source native launcher; evidence of an installed package, applications and commands usable offline, and observed process tree/dependencies. No account, model, or pre-existing index prerequisite for local use. | Full product |
| OBJ-02 | Keyboard-first palette; native tasks demonstrate stable selection, source preview/action, IME, themes, zoom, and screen-reader accessibility on each claimed OS. | Full product |
| OBJ-03 | Find and open the intended source and passage using held-out tasks; bind preview to source/version/location, report exact opening or explicit fallback, and report errors in the denominator. | Full product |
| OBJ-04 | Per-source choices for existing, on-demand, manual, authorized background, or team preparation; persist choices only as selected and offer session scope; test refusal and choice changes. | Full product |
| OBJ-05 | Test all Sillage-controlled destinations, including temporary copies and caches, for absence of query, document path/content, discovered-file catalogue, and derivatives after session/crash; document OS/external limits. Approved-root/grant control state is separately chosen. | Full product |
| OBJ-06 | Measure during typing, compilation, battery use, and low disk; prove pause/resume/cancel, throughput and partial coverage, saturation handling, and reconciliation within interactive and CPU/battery/I/O/RAM/disk budgets. | Full product |
| OBJ-07 | Demonstrate representation migration A→B, interruption and resumption, conditional rollback, revocation/deletion during migration, and preservation of approved APIs/preferences. | Full product |
| OBJ-08 | Installed external client performs search, provenance, read, and cancellation without Slint or generation dependence; errors and versions are documented. | Full product |
| OBJ-09 | Demonstrate real install/activation/removal/revocation, isolated worker, declarative UI, proven denied access, and extension-failure containment without launcher loss. | Full product |
| OBJ-10 | Record an OS/version/architecture/package matrix and run installed native user tasks on Linux, Windows, and macOS; a stub or cross-OS compilation is not qualification. | Full product |
| OBJ-11 | Revalidate authorization/freshness at retrieval, preview, read/release, and action; a known revocation invalidates results, extracts, caches, graphs, and generations, while offline rules/detection delays are displayed. | Full product |
| OBJ-12 | Preserve without omission or weakening every applicable target from ROADMAP's complete `Initial performance and quality acceptance targets` block at the audited SHA; verify hot/cold, latency, memory, CPU, corpus, interactions, paraphrases, and rights. Keep the internal 100 ms deadline distinct; unavailable measurements cannot pass. | Full product |
| OBJ-13 | A non-specialist completes the authorized-source → policy → rights-test → search → reusable-excerpt path without coding or putting administrative keys in the client. | Full product |
| OBJ-14 | Compare optional retrieval by task class against faithful baselines and budgets; unqualified methods remain disabled and lexical success needs no neural promotion. | Research readiness |
| OBJ-15 | Link errata to original claims; verify exact SHAs and artifact availability; preserve negative outcomes and closed scopes; prohibit replay. | Full product and research readiness |
| OBJ-16 | For each claimed experiment, retain the protocol, corpus, all attempts, identities, costs/status, measurement state, and resolvable artifacts; preserve v1 history and actually validate v2. | Full product and research readiness |
| OBJ-17 | For a separately authorized campaign, define a falsifiable question; include negative results, reviewer-regenerated tables, limitations, and licenses; assign no badge or publication automatically. | Conditional research package |
| OBJ-18 | Close against the integrated revision and explicitly chosen profile with actual CI/package evidence; cover its objectives, expose gaps, hide no blocker in a `done` status, and distinguish intermediate release from the full mandate. | Full product |

Full mandate completion requires the full-product and research-readiness
profiles. OBJ-17 is conditional on an independently authorized campaign.
[`ORCHESTRATION.md`](ORCHESTRATION.md) maps these IDs to the kit's proposed
ticket keys and existing issue work without setting status or priority.

## Bootstrap Books

### Book I — Domain Kernel

- fundamental typed IDs, validated values, and invariant-owning primitives
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

This section describes the existing durable local-indexing contract only when
the source policy explicitly selects prepared persistence. It does not make an
index mandatory, define on-demand mode, or weaken the no-document-persistence
requirements above.

For that selected durable profile, the local MVP treats file evidence as immutable source-backed data:

- file-span evidence may reference an immutable blob snapshot;
- snapshot hashes are verified before search hits or opened evidence are returned;
- repeated identical evidence writes are idempotent;
- conflicting evidence writes return a typed storage conflict;
- instance manifests persist approved read roots and privacy exclusion patterns;
- CLI indexing excludes sources outside the persisted read scope before reading bytes
  (counted as policy skips in the batch summary, never read);
- directory indexing is whitelist-first: the choice layer classifies every directory,
  the user approves a whitelist (interactively or non-interactively), and only
  whitelisted files are submitted under their per-directory policy switches;
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
