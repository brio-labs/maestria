# Hybrid Promotion Criteria

This document defines the criteria and evaluation requirements for promoting a hybrid search candidate (v0.5) over the deterministic baseline (v0.4). Consistent with the Maestria philosophy, performance claims must be grounded in measured, typed evidence against a versioned corpus, and the process remains model and backend agnostic.

## 1. Frozen Baseline (v0.4)

The current retrieval baseline is v0.4, providing a conservative, local-first, and purely deterministic search foundation. This frozen baseline establishes the minimum accepted quality, latency, and resource floor. Any hybrid search promotion must demonstrate material improvement over this baseline without degrading privacy, security, or mandatory constraints.

## 2. Shadow-by-Default Policy

The core search path defaults to deterministic baseline execution. New retrieval lanes, including dense or hybrid retrieval, must operate in an explicit shadow mode.
* In shadow mode, the hybrid lane executes and reports its results, observations, and telemetry, but it must exclude dense candidates from the served fusion.
* Active hybrid mode requires an explicit policy opt-in and is granted only after successful evaluation.

## 3. Candidate Evaluation (v0.5)

The v0.5 candidate represents the proposed hybrid search evaluation. A valid comparison between the v0.4 baseline and the v0.5 candidate must:
* Validate the exact same versioned evaluation corpus and query identities.
* Evaluate both observation sets side-by-side through the existing deterministic GoldenGate checks.
* Aggregate all measurements objectively to produce a deterministic comparison report.

## 4. Mandatory Evidence

A promotion decision requires measured, source-grounded evidence. Estimated or fabricated data is strictly prohibited. The candidate evaluation must collect and report:
* **Quality**: Relevance judgments against the versioned judgment set.
* **Material improvement**: at least five percentage points on one aggregate quality metric (the fixed-point `Metric` scale uses 10,000 = 100%).
* **Latency**: Computed p50, p95, and p99 percentiles derived from caller-supplied, per-query execution samples.
* **Resources**: Measured memory and disk consumption footprints.
Resource telemetry is explicit: latency, memory, and disk zeroes are measured values only when the observation marks resource telemetry complete.
Security telemetry is explicit: zero leakage or attack counts are measured zeros, not an implicit default. An observation missing security telemetry fails its gate.
* **Privacy & Security**: Proof that all scope, ACL, trust, sensitivity, quarantine, and prompt-injection checks are applied before scoring or exposing candidates.
* **Energy**: Measured energy usage data. If energy telemetry is unavailable or cannot be measured truthfully, it must be explicitly recorded and preserved as `None`.

Ingest and update cost must also be measured for the same corpus and update workload. Missing telemetry remains explicitly unavailable and cannot support a promotion claim.
Promotion configurations must also specify explicit ingest/update and energy budgets. A comparison without those frozen budgets retains the deterministic baseline.

## 5. Backend Tier Criteria

Tier selection is a measured deployment decision, not a model preference:

| Tier | Corpus and workload | Required evidence |
|---|---|---|
| S | Small local corpus and interactive workstation budget | Deterministic quality gate, p95 latency, peak memory/disk, privacy/security checks, and reproducible incremental-update cost. |
| M | Medium multi-project corpus with concurrent updates | The S measurements plus p99 latency, sustained ingest throughput, restart/recovery behavior, and bounded projection growth. |
| L | Large corpus or shared high-throughput deployment | The M measurements plus load-saturated latency, failure/retry behavior, migration/rollback evidence, energy telemetry, and isolation under concurrent tenants. |

An implementation may not claim a higher tier because it uses a more advanced backend. The dated comparison must identify the tier, corpus snapshot, index generation, workload, and measured limits.

## 6. Conditional ANN Benchmarking

The candidate must utilize exact search mechanisms by default. Approximate Nearest Neighbor (ANN) benchmarking and indexing are conditional. They are permitted only after demonstrating a measured `sqlite-vec` budget failure under the deterministic baseline budget.

## 7. Explicit Promotion Records

A promotion from shadow to active mode must not be silent, implicit, or autonomous. The transition requires an explicit promotion decision record that includes:
* A caller-supplied evaluation ID.
* The date of the evaluation.

These records must be preserved to ensure the factual answer path remains auditable and that promoted behaviors point back to their authorizing evidence.
