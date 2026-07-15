# Book IV — Ecosystem, Storage, and Projections

The ecosystem and storage layer provide projections and I/O-backed capabilities, not
authoritative state or external factual authority. Domain state remains in the kernel
model, while evidence preserves observations without asserting that they are true.

## Purpose

- Store event logs, blob content, full-text indexes, and parser-derived artifacts.
- Expose stable adapters implementing `maestria-ports` contracts.
- Keep projection semantics explicit and deterministic where possible.

## Core Rules

1. `I-Storage-ProjectionOnly` — projections support, not replace, domain truth.
2. `I-DTO-Boundary` — boundary DTOs remain isolated from domain primitives.
3. `I-Dependency-Layered` — avoid cyclic and backwards dependencies into kernel mechanism.
4. `I-Ingestion-Idempotent` — unchanged source content is a no-op, and incomplete ingestion is retryable without false completion.
5. `I-Task-Workspace` — task workspace directories are prepared before task persistence.

## Interfaces and boundaries

- Storage crates implement port contracts.
- Parsers and validators are deterministic adapters.
- Validation and memory services stay policy-aware and typed.

## Verification

- Contract tests for adapters in their port-owned test suites.
- Integration replay paths for persisted projections.
