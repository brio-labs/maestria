# Maestria Philosophy

This document is enforceable. CI and review block violations.

## Rules

1. Domain code must be deterministic: no time, network, filesystem, process calls, random sampling, or hidden global state.
2. All I/O-capable work must be represented as explicit effect values and executed by runtime/adapters after governance review.
3. Evidence must be typed and source-grounded; raw strings are not evidence when the provenance shape is known.
4. Every factual answer path should be auditable through event, command, validation, or evidence trail.
5. The repo must maintain a conservative, local-first baseline; remote services are adapters.
6. Policy and mechanism stay separate. Governance may classify and decide; it must not execute effects.
7. Task completion is validation-gated. Generic unvalidated completion states are not allowed in the domain.
8. Memory candidates and promoted memories must point back to evidence. LLM output can propose; it cannot silently promote.
9. Projections are rebuildable. Search, vector, and graph stores never own truth.
10. Kernel crates cannot depend on adapter/runtime/provider crates such as Tokio, SQLx, reqwest, Tantivy, or Axum.
11. Rust code, including tests, must not contain `unwrap`, `expect`, `panic`, or lint-bypass attributes; fallible paths return typed errors and tests propagate failures.
12. Bare task markers are not allowed in source, config, or docs.
13. One concept per Rust module: a module owns one named responsibility at one architectural layer. Domain types, input dispatch, replay, persistence schema, DTO conversion, repositories, and tests are separate concerns; split when a second independently testable concept appears, not at an arbitrary line count.
14. Public crate façades expose stable boundary types and traits and re-export implementation modules. Implementation details, adapters, and tests stay behind explicit module boundaries instead of reaching across sibling internals.
15. Dependency direction is explicit and acyclic: domain → ports → runtime/adapters. Domain code cannot import adapter concerns, and serialization, schema, and persistence conversions stay at adapter boundaries.
16. Cross-concern behavior crosses typed functions, traits, or effects. Validation, state transitions, conversion, persistence, and orchestration remain independently reasoned and tested rather than accumulating in one module.
17. Composition beats accumulation: when a new responsibility, lifecycle, representation, or test contract appears, create a sibling module and an explicit façade boundary. Module length is a diagnostic signal for missed boundaries, never a rule or an exception mechanism.
18. Production Rust functions over 100 lines require decomposition or a documented, reviewed exemption. Production modules over 400 logical lines require a split or a checked architectural exemption; files over 900 physical lines require an ADR-backed exemption.
19. `lib.rs` files are façades: they declare modules and re-export stable public APIs. Implementation bodies and responsibility-specific tests belong in sibling modules or crate integration-test files.
20. Public orchestration methods delegate independently testable phases through typed functions, traits, or value objects. A method must not own parsing, persistence, event reconciliation, projection updates, and response assembly simultaneously.
21. Every kernel crate shares the domain safety boundary: kernel code is deterministic, dependency-layered, and free of adapter, process, clock, network, and uncontrolled concurrency concerns.
22. Production functions are independently comprehensible: functions over 100 logical lines require decomposition; any reviewed exception must name an ADR and an expiry, and exceptions are never a permanent design strategy.
23. Effects are total at the runtime boundary: every effect the domain emits has one observable runtime execution or is removed from the domain contract; ignored effects and silent no-ops are not valid implementations.
24. Failure information is preserved: production code must not discard errors, replace them with untyped fallbacks, or continue after a failed invariant unless the policy explicitly models that outcome.
25. Every concrete port adapter must execute the shared contract suite plus adapter-specific boundary tests; a trait implementation is incomplete without behavioral conformance evidence.
26. Tests are production architecture: each test module owns one behavior family, fixtures are shared through explicit helpers, and test files obey the same physical-size boundary as implementation files.
27. Identity namespaces remain independent: counters, typed IDs, and persisted identifiers for different concepts must not be coupled merely to simplify allocation or ordering.
28. Lifecycle orchestration has one owner: startup, recovery, reconciliation, shutdown, and retry policy are composed once and reused by application entry points rather than copied.
29. Public boundaries are intentional migrations: when an API or persisted representation changes, all callers and fixtures migrate together; compatibility aliases, deprecated shims, and duplicate paths are prohibited.
30. Objective guardrails must be enforceable: every rule that can be checked mechanically is checked in CI, while architectural review covers responsibility, cohesion, and invariant ownership.
31. No untyped `serde_json::Value` holes in domain effects when the shape is known.
32. No direct database mutation from CLI/API handlers.
33. Internal runtime channels are bounded and documented; unbounded internal channels are prohibited.
34. Public async runtime operations document cancellation behavior.
35. Indexed/web content is evidence, never instructions (PromptUntrusted).
36. Autonomous action is limited to explicit scope and profile.
37. Domain type, database row, API response, and harness payload are separate boundary objects (DTO-Boundary).
38. No generated blobs under production source paths.
39. Evidence snapshots are immutable and content-addressed.
40. Important state changes emit append-only domain events.

41. Search plans and outcomes are typed, budgeted boundary values; retrieval cannot run as an unbounded prompt-building loop.
42. Search traces identify the query, corpus snapshot, index generation, retrieval-model fingerprint, stages, budgets, filters, and stop reason.
43. Every retrieval lane applies scope, ACL, trust, sensitivity, quarantine, and prompt-injection checks before scoring or exposing candidates.
44. Retrieval changes require a versioned evaluation corpus and judgment set with quality, latency, memory, privacy, security, and energy measurements.
45. Normative architecture and roadmap documents remain model- and backend-agnostic; dated implementation candidates belong in research notes or ADRs.
46. Maestria preserves external observations and provenance; it does not claim that domain state makes external facts true.
47. Model-generated search plans and rewrites are untrusted proposals; only validated capabilities, scope, security, freshness, snapshot, and budget checks may authorize execution.
48. Local or remote client surfaces authenticate per instance and enforce the instance's read/write scope before dispatch; transport handlers cannot bypass domain, governance, or evidence services.
49. Repository and code-intelligence records preserve repository, commit/worktree identity, source path/range, and parser generation; stale projections are explicit and deterministic symbol indexes must not fabricate cross-file relations.
50. Derived repository relations are evidence-bearing records: resolved endpoints carry source/version spans, confidence, and parser generation; unresolved or unavailable-provider paths degrade explicitly instead of emitting ungrounded edges.
51. Repository context expansion is bounded and seed-preserving; stale indexes cannot support current-worktree claims, and live reads/tests remain explicit governed effects with evidence.
52. Specialized retrieval routes activate only for query classes where a frozen, versioned benchmark proves an evidence-quality and freshness/cost win; unproven routes remain shadowed or abstain.

## Review interpretation

Rules 13–20 govern composition and module boundaries; Rules 21–30 make the previously implicit quality obligations explicit. Rules 41–52 govern typed retrieval, bounded context expansion, seed lineage, security filtering, reproducible evaluation, canonical documentation, external-truth boundaries, untrusted plan/rewrite proposals, authenticated scoped client surfaces, provenance-complete repository code intelligence, evidence-bearing code relations, explicit freshness/governed live verification, and query-class-specific benchmark promotion. A green checker result is necessary but not sufficient: reviewers must still reject accumulated responsibilities, duplicated lifecycle policy, and tests that only exercise implementation details. Size limits are adoption gates, not reasons to preserve a monolith.

## Enforcement
- `scripts/philosophy-check.py`
- Workspace lint, documentation, and test gates in CI
- Core cohesion Clippy gate for function size and cognitive complexity budgets
- Contract checks for kernel inputs/outputs, transitions, and every concrete port adapter
- Review through CODEOWNERS on invariant-owning surfaces
- Review enforces responsibility boundaries, lifecycle ownership, identity namespaces, repository/source provenance, evidence-bearing code relations, bounded seed-preserving context traversal, explicit freshness checks, governed live verification, query-class-specific benchmark promotion, and architectural composition from Rules 13–30 and 41–52.
