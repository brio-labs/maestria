# ADR-0001: Modular Monolith

## Context

We need fast iteration on domain rules while preserving replaceable boundaries.

## Decision

Adopt a modular-monolith workspace with independently versioned kernel crates and adapters.

## Consequences

- Shared contracts remain stable.
- Boundaries are explicit and testable.
- Operational complexity stays low in early milestones.
