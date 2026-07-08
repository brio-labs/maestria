# Maestria Initial Specification Ledger

## Invariant 1: Deterministic Kernel

Given the same ordered input stream, initial state, and policy snapshot,
the kernel must produce the same events and final state.

## Invariant 2: Explicit Effects

Domain code emits effects as values; execution of side effects occurs only in runtime.

## Invariant 3: Source-Grounded Evidence

Every claim introduced into memory must be paired with evidence metadata including provenance.

## Invariant 4: No Hidden Mutations

Kernel functions must not mutate external systems directly.

## Invariant 5: Replayability

Domain events and state transitions must support deterministic replay from recorded input history.
