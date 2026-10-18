# Sillage Orchestration Mandate

This document governs execution ownership, evidence, and stop decisions for
work under the adopted Sillage launcher-retrieval mandate. It does not set
product priority, milestone order, or dates. [`ROADMAP.md`](ROADMAP.md) is the
sole planning and priority authority; GitHub issues and pull requests are the
live work-state authority. This document is not a second schedule or a manual
status ledger.

## Authority and document ownership

The adopted mandate is *Cahier des charges launcher retrieval*, version 0.4,
dated 7 October 2026, read with its structured kit.
[`SPECS.md#mandated-product-objectives`](SPECS.md#mandated-product-objectives)
is the canonical authority for all 18 objective outcomes, evidence, and
completion profiles. The crosswalk below maps those IDs to proposed kit
tickets and existing issues; it does not redefine acceptance or record live
status. The audit snapshot `af94c764652526c66f1cca199a6cf879b069d048` was
static and read-only. The kit's `acceptance/requirements.json`,
`planning/new_tickets.json`, `planning/existing_issue_dispositions.json`, and
`planning/milestones.json` are source mappings, not live status. Its 21 local
ticket keys are proposals, not GitHub issue numbers. The 44 reviewed
existing-issue dispositions retain their identities, histories, and first
outcomes; use existing issues and the [`#559` execution umbrella](https://github.com/brio-labs/maestria/issues/559),
not duplicate tickets.

Canonical ownership is singular:

- [`PHILOSOPHY.md`](PHILOSOPHY.md): enforceable repository doctrine.
- [`SPECS.md`](SPECS.md): system requirements and the canonical 18-objective
  acceptance matrix.
- [`ARCHITECTURE.md`](ARCHITECTURE.md): logical system and component boundaries.
- [`ROADMAP.md`](ROADMAP.md): only product priority, milestone order, and current delivery status.
- [`RESEARCH.md`](RESEARCH.md): dated research context, evidence interpretation, and candidates; no product priority.
- This document: assignment, ownership, evidence and stop/closure procedure only.
- [ADR-0011](adr/ADR-0011-source-retention-public-api.md): adopted source-retention and public-API doctrine; implementation remains pending.

The mandate supersedes only conflicting doctrinal clauses recorded in
[ADR-0011](adr/ADR-0011-source-retention-public-api.md). It does not
retroactively satisfy, alter, or replay old acceptance criteria or evidence.
ADR-0009 (retrieval audit retention) and ADR-0010 (Studio proxy topology)
remain **Proposed**; adopting this doctrine does not accept either decision
or implement it.

## Objective-to-work-item crosswalk

SPECS defines the required objectives, profiles, and evidence. The crosswalk
is execution traceability only; ticket association does not make optional
work a launch dependency or establish a completion status.

| Objective | Kit ticket keys / existing issue anchors |
|---|---|
| OBJ-01 | #519, #523, #538; REL-01 |
| OBJ-02 | #520–#522, #549, #557, #558 |
| OBJ-03 | #526–#528, #530, #531, #534; SCI-02 |
| OBJ-04 | SRC-01–SRC-03, #525; ENT-01 |
| OBJ-05 | SRC-01, SRC-02; SEC-01 |
| OBJ-06 | IDX-01, #525; SCI-02 |
| OBJ-07 | IDX-02; API-01; SEC-01 |
| OBJ-08 | API-01, API-02, #524, #529 |
| OBJ-09 | #535–#537, #556; OS-01 |
| OBJ-10 | OS-01–OS-03, #547–#549; REL-01 |
| OBJ-11 | SEC-01, #529; API-02; IDX-02 |
| OBJ-12 | CI-01, #534, #538; REL-01 |
| OBJ-13 | ENT-01, API-02; SEC-01 |
| OBJ-14 | RET-01, RET-02, #533, #92–#95 |
| OBJ-15 | EVD-01, GOV-01, #60 |
| OBJ-16 | SCI-01–SCI-03 |
| OBJ-17 | SCI-03 |
| OBJ-18 | GOV-01, CI-01; REL-01; #559 |

The 44 existing-issue dispositions remain in the kit's structured ledger and
retain their GitHub identities and histories. The eight proposed milestones
are not restated here: any adopted sequencing belongs only in ROADMAP.

## Integration branch and release state

Verified GOV-01 slices integrate into `dev/sillage`. `main` remains **Shadow**,
PR #516 remains **draft**, and product qualification remains **false**. This is
execution-state information, not an alternate planning authority or permission
to publish or qualify; [`ROADMAP.md`](ROADMAP.md) remains the sole priority and
milestone authority, and GitHub issues/PRs remain the live work-state authority.

## Assignment and ownership

Every assignment identifies the objective(s), full starting revision, owned
paths and interfaces, known dependencies, authorized data/scope, resource
limits, deliverables, and required evidence. Assign one owner to each shared
interface before parallel work. Work in the assigned isolated source worktree;
do not edit another owner's paths, frozen candidates, protected roots, or
evidence. Preserve unrelated user changes. Do not make GitHub, publication,
deployment, or external-spend changes without explicit authority.

Implementers update all affected callers and contracts within the owned slice
and report what changed, what remains, and what was not exercised. Reviewers
assess actual behavior and evidence at the relevant boundary; a code review,
compile result, issue closure, merge, or historical report is not product
qualification. Integration verifies combined changes once at the integrated
revision. A task may be split by objective or contract, but acceptance
obligations may not be silently reduced.

## Evidence, privacy, and scope

Every result is bound to the exact revision, artifact, OS/version/architecture,
profile/source scope, procedure, and evidence actually inspected. Distinguish
source inspection, automated contract checks, installed-product exercise,
research exploration, and confirmatory evidence. Record `not run`,
`unavailable`, `failed`, `incomplete`, or `blocked` as such; never convert
missing evidence into a pass or a zero. A compatibility or performance claim
is limited to the tested configuration and task.

Preserve first outcomes, failures, negative results, and their identities.
Never replay a frozen/closed historical campaign or consume reserved inputs.
A new protocol, model, corpus name, budget, repair, or commit does not itself
make a scope new. Before a new experiment, establish its independently
authorized scope and unused inputs. Regenerating a table from retained
artifacts is not replaying the underlying experiment. Ordinary regression
fixtures remain usable under their own contract.

Do not publish private originals or disclose private query/title/URL/clipboard
values, credentials, raw logs, or identifying source data. Ordinary telemetry
export is opt-in. Evidence collected for a voluntary experiment requires
separate, explicit consent and must be limited to its authorized purpose and
retention. Missing authorization, private artifacts, hardware, operating-
system capability, or budget is a named prerequisite, not inferred
availability or permission.

## Stop, resume, and close

Stop the affected action before it crosses an unapproved source, network,
privacy, security, budget, or irreversible boundary; before accessing
consumed/reserved evidence; or when an invariant, scope check, authorization,
or required capability cannot be verified. Preserve observed errors and
partial outcomes. Do not weaken a gate, widen scope, silently retry
non-idempotent work, or infer permission from a related grant. Report the
precise blocker and evidence needed to resume; continue independent authorized
work without hiding the blocked item.

A task is done only when every mandatory criterion for its declared profile is
covered on the integrated revision and remaining limitations are explicit.
An unqualified OS or product path stays unqualified. Report changed paths,
exact revision/artifacts, exercised commands and results, unexercised checks,
failures, limits, and the next real dependency. Verification not performed
must be named, never represented as passed.
