# Book III — Runtime and Shell Boundary

The runtime converts declarative domain intent into external effects.

## Purpose

- Own asynchronous orchestration.
- Bound channels, queue behavior, and cancellation points.
- Execute adapters (`harness`, `storage`, `parsers`) behind typed interfaces.
- Feed results back to the domain as typed inputs.

## Core Rules

1. `I-Runtime-BoundedChannels` — bounded channels with explicit pressure policy.
2. `I-Runtime-CancelSafe` — cancellation behavior is visible and documented.
3. `I-Effect-Explicit` — effects are values in domain output.
4. `I-Harness-NoTruth` — adapters report outcomes; they do not arbitrate truth.

## Recommended structure

- Single effect loop with clear effect mapping and worker supervision.
- Deterministic assignment of IDs before asynchronous execution.
- Replay safety for startup, restart, and retry paths.

## Verification

- Tests for effect scheduling, retry semantics, and deterministic replays.
- CI gates include checks for linting, docs, tests, and philosophy checks.
