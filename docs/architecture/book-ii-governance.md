# Book II — Governance and Policy

Governance is where policy decisions live. It evaluates intent, risk, and context, then returns
structured results the domain/runtime can act on.

## Purpose

- Encode policy and permission decisions without mutating domain truth.
- Keep policy changes discoverable and separately reviewable.
- Provide trait-based interfaces for autonomous behavior and safety boundaries.

## Non-goals

- Not a place for storage orchestration.
- Not a place for command execution.
- Not a place for adapter implementation details.

## Core Rules

1. `I-Policy-BeforeAction` — risky actions require explicit policy result.
2. `I-Memory-SourceBacked` and `I-Memory-CandidateGate` — memory promotion is a policy-aware path.
3. `I-Scope-ExplicitAutonomy` — autonomy controls must be explicit and reviewable.

## Recommended structure

- Trait-based approvals and checks.
- Input validation that is deterministic with explicit decisions.
- Minimal coupling: consume domain primitives; avoid runtime dependencies.

## Verification

- Unit and scenario tests around refusal, approval, and fallthrough behavior.
- Policy test fixtures aligned with [SPECS.md](../SPECS.md).
