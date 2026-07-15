# Memory Architecture

This document defines the durable contract for Maestria's memory architecture. It outlines the lifecycle of information from initial observation to long-term storage or deprecation.

## 1. Information Lifecycle

The system treats information through a strictly phased lifecycle:

*   **Observation:** A source-backed observation or user-provided signal. Observations retain origin and acquisition context but are not conclusions.
*   **Candidate:** A structured proposition derived from observations. Candidates must include complete provenance and an explicit uncertainty/status.
*   **Typed boundary:** The domain representation for a candidate is `MemoryCandidate`; promotion produces a governed `Memory` value.
*   **Evidence/Review:** Candidates are evaluated against immutable evidence, existing knowledge, project constraints, and user policy. Supporting and conflicting evidence are recorded.
*   **Promotion:** A candidate becomes active working knowledge only after evidence and a promotion decision satisfy the applicable policy. Promotion never establishes universal external truth.
*   **Deprecation/Contradiction:** Active memories that conflict with newer evidence or are explicitly invalidated are deprecated, contradicted, or superseded. Historical records remain auditable and are excluded from active synthesis according to policy.

## 2. Provenance and Staleness

*   **Provenance:** Every promoted memory MUST retain exact lineage. This includes the source document, user interaction, or derived reasoning step that created it.
*   **Staleness:** Memories are subject to temporal decay. Facts related to volatile context (e.g., "current working branch") decay rapidly, while foundational facts (e.g., "project language is TypeScript") decay slowly or not at all.
*   **Revalidation:** Stale memories accessed during critical operations trigger revalidation protocols before use.

## 3. Boundaries and Overclaiming

*   **Internal Truth:** Memory represents the *system's understanding* of the project and user, not universal truth.
*   **No External-Truth Overclaiming:** The memory system MUST NOT present synthesized conclusions as absolute facts unless corroborated by explicit user confirmation or deterministic codebase evidence.

See [OPERATIONS.md](./OPERATIONS.md) for how these states are persisted and recovered.
