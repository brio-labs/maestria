# ADR-0004: Effect Inversion

## Context

Side effects were previously implicit and hard to audit.

## Decision

Introduce explicit effect values produced by domain and consumed by runtime.

## Consequences

- Deterministic replay.
- Better validation and approval gates.
- Easier substitution of effect handlers.
