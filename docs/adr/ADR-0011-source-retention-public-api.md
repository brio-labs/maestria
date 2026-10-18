# ADR-0011: Source Retention and Public API Boundaries

## Status

**Accepted for doctrine; implementation and qualification pending.** Adopted under GOV-01 from the Sillage launcher-retrieval mandate, version 0.4, dated 7 October 2026. This ADR records requirements; it does not assert that source modes, portable clients, API transitions, or operating-system support have been implemented or qualified.

## Context

The mandate requires a native retrieval launcher and reusable search boundary on Linux, Windows, and macOS. It also requires per-source choices, including retrieval without persistent document derivatives, while existing doctrine requires append-only domain events and traceable retrieval. The existing API clean-cutover rule does not define public protocol version negotiation. These obligations must be reconciled without making a model, account, prebuilt index, or generation/chat capability mandatory, without turning retrieval traces into durable query histories, and without silently permitting compatibility shims.

The current architecture and runtime are a starting point, not evidence that these requirements are met. Existing Linux proofs remain bounded to their named configurations and tasks; Windows and macOS remain unqualified. The audit snapshot at `af94c764652526c66f1cca199a6cf879b069d048` was static and did not execute product behavior.

## Decision

### Product and platform boundary

Sillage's product core is an open-source native retrieval launcher and reusable retrieval API. The required target platforms are Linux, Windows, and macOS. Installed use of the launcher and its local app/command path must not require a user account, model, or pre-existing document index. Generation and chat are optional capabilities exposed through plugins or API clients, not a prerequisite or the product's required search response. Refusing index preparation remains a useful bounded retrieval path; it is not an exhaustive, instant semantic scan of the entire computer. Partial coverage and stop conditions are explicit.

Mandated operating-system support and qualification are separate. A platform is not qualified by source portability, a stub, CI compilation, or success on another OS. Qualification names the actual OS version, architecture, package and exercised user tasks. Linux evidence is only as broad as its inspected evidence; Windows and macOS are not qualified by this decision.

### Independent per-source policy

A source's scope, execution place, computation timing, documentary retention, representation choices, and resource budgets are independent policy dimensions. A user-facing profile is a comprehensible combination of those choices, not a hidden coupling that makes a local source remote, requires an index for on-demand search, or forces one representation. The policy survives retrieval-engine replacement; no exact API, storage schema, or wire format is prescribed here.

Source scope is explicit and deny-by-default. Approved local roots, consumer/provider grants, and remote-service authorization are distinct controls: local approval never implies remote disclosure or a provider grant. Authorization and source-version freshness are rechecked before dispatch, reading, releasing a result, previewing, and taking an action. Empty scope denies. A partial scan must show its coverage; no result does not establish that nothing exists outside that coverage.

Preferences and control state (including approved-root manifests and grants) are not documentary content. Their persistence is separately disclosed and chosen; session-only control is available when a choice must not be remembered. A persisted root or grant is only this separately consented control state; it does not authorize retaining a query, document path, or source content.

### No-document-persistence and observability

When a source's policy prohibits documentary persistence, no Sillage-controlled store may persist its query, document paths, discovered-file catalogue, document content, or derivative. An approved-root manifest or grant may persist only as separately chosen control state. The prohibition applies end to end to lexical/vector/graph projections, chunks, summaries, OCR, thumbnails, provider caches, temporary files, logs, crash/recovery state, and query-linked traces. Avoiding embeddings alone does not satisfy the policy. The restriction is scoped per source and does not erase independently authorized control settings.

Rule 40 requires append-only events for important domain state changes; it does not require a durable event for every search. Search plans, outcomes, and traces remain typed and budgeted under Rules 41–42, but their operational traces are ephemeral by default and live only as long as the authorized session needs them. Traces identify query and retrieval context for that bounded execution without creating persistent query history. No durable search trace or ordinary telemetry export is implied by the event or trace rules.

Ordinary telemetry export is opt-in. Evidence gathered for a voluntary experiment requires a separate, explicit consent and purpose; ordinary user sessions do not silently become research data. Experiment records retain only the evidence authorized for that experiment and its stated retention. Application-controlled files and stores must honor the selected policy. Sillage cannot promise absolute erasure from operating-system caches, swap, external-provider logs, or backups it does not control; those limits must be disclosed and must not conceal residual data in Sillage-controlled stores.

### Retirement and purge are distinct

Pausing a source or maintenance stops work; disabling or withdrawing a source makes it unavailable under policy; retiring a representation removes it from serving eligibility. None of those actions means its bytes or records have been erased. Physical purge is a separate, explicit, authorized lifecycle action that addresses controlled representations, caches, temporary data, and references according to the selected retention policy. When policy requires deletion, remove the controlled artifact and state the resulting absence and verification limitation without silently rewriting append-only domain events. Retention or deletion of domain events remains a separate governed decision.

ADR-0009, [Retrieval Audit Event Retention](ADR-0009-search-knowledge-retention.md), remains **Proposed** and pending review; its proposal covers governed retrieval-audit retirement markers, not physical purge, and that scope remains unapproved and unimplemented. This ADR neither accepts its mechanism nor implements retirement or purge.

### Public API version transitions

Public retrieval contracts may expose intentional version negotiation. A dated, bounded transition between explicitly identified public versions may be authorized only by a separately approved ADR that defines its scope, supported versions, migration path, end date, and removal criterion. This is not blanket authorization for compatibility code. Permanent aliases, deprecated shims, and duplicate paths remain prohibited. Internal APIs and persisted representations use clean cutover: migrate all callers and fixtures together, with no compatibility alias or standing fallback. Exact APIs and wire schemas remain the responsibility of their versioned implementation specifications.

ADR-0010, [Studio Proxy Topology](ADR-0010-studio-proxy-topology.md), remains **Proposed**; its proposed Studio/daemon topology scope remains pending review and implementation. This doctrinal adoption does not accept or implement it.

### Canonical identity and historical records

Sillage is the canonical product identity. No implicit Maestria executable, settings, grants, credentials, index, model assets, or other data migration is authorized. The explicit identity cutover and no-alias/no-migration rule remain in [`ROADMAP.md`](../ROADMAP.md#milestone-1-slint-launcher-and-native-experience). Legacy scientific reports and observations retain their historical Maestria labels, revision identities, outcomes, access limits, and replay restrictions; this ADR does not rename, revise, qualify, republish, or replay them. [`RESEARCH.md`](../RESEARCH.md) remains the owner for research history and dated interpretation.

### Superseded-clause provenance

These links pin the pre-adoption text to the static audit baseline
`af94c764652526c66f1cca199a6cf879b069d048`. Rule, invariant, and section names
identify the relevant clauses without guessing historical line anchors. This
provenance explains the adopted interpretation; it does not rewrite or
invalidate the historical text.

- [`PHILOSOPHY.md` at the baseline](https://github.com/brio-labs/maestria/blob/af94c764652526c66f1cca199a6cf879b069d048/docs/PHILOSOPHY.md): Rule 29's clean-cutover and no-alias requirements, Rule 40's append-only important-state events, Rule 41's typed/budgeted search plans and outcomes, and Rule 42's query/context trace identity. Internal APIs and persisted representations retain clean cutover and permanent aliases/shims remain prohibited; a bounded public-version transition requires a separately approved ADR. Rule 40 applies to important domain state changes, not every operational search; Rules 41–42 require typed, budgeted diagnosis during the authorized execution lifetime, not durable query histories.
- [`SPECS.md` at the baseline](https://github.com/brio-labs/maestria/blob/af94c764652526c66f1cca199a6cf879b069d048/docs/SPECS.md): `Initial Invariant Set` entries `I-Event-AuditTrail`, `I-Search-TypedBudgeted`, and `I-Search-TraceFingerprint` retain their event-integrity, typed/budgeted-contract, and trace-identity meanings. Under this ADR, operational trace identity is ephemeral by default; retained experiment evidence requires separate authorization and consent.
- [`ARCHITECTURE.md` at the baseline](https://github.com/brio-labs/maestria/blob/af94c764652526c66f1cca199a6cf879b069d048/docs/ARCHITECTURE.md): §9.2, “Search Invariants,” items 8–9, and §15, “Observability, Replay, and Evaluation,” entry “search traces.” They describe trace diagnosis and its non-authority over domain state; this ADR interprets the trace as runtime evidence within its authorized lifetime, not a requirement for durable query-linked storage.

## Consequences

- Source policy must be represented and enforced independently of any particular model, index, launcher, account, or generation client.
- End-to-end no-persistence behavior and the distinction between control state, documentary data, telemetry, and voluntary research evidence require implementation and direct evidence before a corresponding claim is qualified.
- Linux, Windows, and macOS are all mandatory product targets, but each requires its own bounded installed qualification; this ADR supplies none.
- Search traces satisfy retrieval diagnosis during their authorized lifetime without becoming mandatory durable audit rows. Durable domain events remain append-only for actual state changes.
- Retirement cannot be described as deletion or purge. Proposed ADR-0009 and ADR-0010 remain Proposed; status must change through their own review, not by reference here.
- Public API transitions require explicit approval and a removal date; internal changes remain clean cutovers. No exact APIs, transition implementation, or compatibility path is authorized by this decision.
