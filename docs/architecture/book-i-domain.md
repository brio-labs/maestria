# Book I — Domain Kernel

Maestria's domain is the source of truth. It contains **typed state**, **invariant-preserving
transitions**, and **pure deterministic logic**.

## Purpose

- Model artifacts, claims, tasks, evidence, validations, memory candidates, and events.
- Expose state transitions that never directly execute external side effects.
- Emit effect descriptors when asynchronous work is needed.

## Non-goals

- No process, filesystem, networking, or wall-clock reads.
- No adapter-specific types.
- No persistence strategy concerns.

## Core Rules

1. `I-Domain-Pure` — transitions only inspect/update in-memory state.
2. `I-Domain-NoPanic` — production domain code avoids panic paths.
3. `I-Event-AuditTrail` — critical transitions are replayable through events.
4. `I-Dependency-Layered` — domain crates do not depend on runtime/storage adapters.

## Public API style

- `pub` items document behavior and invariants.
- Changes that affect proofs of correctness must update [SPECS.md](../SPECS.md).
- Domain errors and fallbacks are explicit.

## Interfaces

- `maestria-domain` exposes core models and deterministic transition functions.
- `maestria-governance` depends on kernel capabilities, not vice versa.

## Verification

- Unit tests for transition behavior.
- Invariant-focused checks in `scripts/philosophy-check.py`.
- Future work: replay and property test suites under `tests/replay` and `tests/property`.
