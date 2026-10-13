# Book IV — Ecosystem, Storage, and Projections

The ecosystem and storage layer provide projections and I/O-backed capabilities, not
authoritative state or external factual authority. Domain state remains in the kernel
model, while evidence preserves observations without asserting that they are true.

## Purpose

- Store event logs, blob content, full-text indexes, and parser-derived artifacts.
- Expose stable adapters implementing `sillage-ports` contracts.
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

- The public, opt-in bounded Markdown path takes borrowed `FileHandleRef` input
  `Parser::parse_outcome_bounded` and explicit `ParseBudget` work/output charges.
  It builds the canonical artifact output, checks cancellation and deadlines
  cooperatively, and declines unsupported bounded inputs without an unbounded
  fallback. Budget usage is logical accounting, not RSS measurement or a total
  time-complexity guarantee.
- Bounded routing and fallback titles share budget-interleaved native path
  component scanning for normalized filenames/extensions, including Windows
  prefix/root semantics; summary display decodes WTF-8 surrogates as one
  replacement without copying path metadata.
- Validation and memory services stay policy-aware and typed.

## Verification

- Contract tests for adapters in their port-owned test suites.
- Integration replay paths for persisted projections.
- Prior isolated parser/ports/domain package verification passed 337 tests across
  46 suites. The public bounded-parser consumer exited successfully after exercising
  Unicode/CRLF/EOF output, canonical spans/cards/hash/version, work/output refusal,
  unsupported no-fallback, and mid-parse cancellation/deadline. Those checks
  predate the current path-fidelity and Clippy corrections and establish component
  behavior only; they do not qualify GUI/installed flows, physical-memory or
  process-tree RSS, or a linear/total-time complexity bound.
- The private deterministic WTF-8 decoder regression exercises the Windows
  surrogate-byte branch on the current host; it is not native Windows runtime
  evidence.
