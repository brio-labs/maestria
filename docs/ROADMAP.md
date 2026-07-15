# Implementation Roadmap

This document serves as the single canonical implementation roadmap for the Maestria search architecture.

The roadmap is phased. Advancing to a subsequent phase requires meeting all observable exit criteria of the current phase.

## Phase 1: Deterministic Baseline
Establish a robust, entirely deterministic foundation for exact and structure-aware search.
*   **Goals:** Exact identifiers, lexical candidates, source spans, provenance, and reproducible traces.
*   **Exit Criteria:**
    *   A versioned Maestria evaluation corpus reports exact-match precision/recall and latency against an approved baseline.
    *   Search results map to immutable source versions and spans, with no scope or ACL leakage.
    *   Index mutations are journaled and projections can be rebuilt from authoritative state.

## Phase 2: Hybrid Search and Reranking
Introduce additional candidate lanes only when they improve the deterministic baseline for a measured query class.
*   **Goals:** Fuse independently testable lexical and semantic candidates, then apply bounded reranking and evidence diversity.
*   **Exit Criteria:**
    *   Versioned evaluation reports show a statistically meaningful quality improvement for the target query set within declared latency, memory, privacy, security, and energy budgets.
    *   Reranking, fusion, and model/index fingerprints are recorded in reproducible traces.
    *   Pluggable adapters allow swapping candidate implementations without core-system changes.

## Phase 3: Adaptive Planning
Implement dynamic query routing and bounded multi-stage retrieval.
*   **Goals:** Select deterministic, lexical, semantic, graph, temporal, code, or visual lanes by typed intent and available budget.
*   **Exit Criteria:**
    *   Routing and decomposition are evaluated against a versioned judgment set and never bypass scope, trust, or ACL filters.
    *   Every plan has explicit budgets, stopping conditions, evidence coverage, and abstention behavior.
    *   Implementation of the observation/candidate/promotion lifecycle detailed in [MEMORY.md](./MEMORY.md).

## Phase 4: Code Intelligence
Deep structural and contextual understanding of codebases.
*   **Goals:** Cross-file dependency resolution, type-aware search, and semantic similarity of code blocks.
*   **Exit Criteria:**
    *   System can accurately map function call graphs across language boundaries where supported.
    *   Semantic search surfaces logically equivalent implementations, not just textually similar ones.

## Phase 5: Visual Documents
Expand search capabilities to include UI and visual assets.
*   **Goals:** Retrieve and understand images, diagrams, and UI layouts relevant to codebase queries.
*   **Exit Criteria:**
    *   Multimodal retrieval successfully maps visual components to their source code implementations.

## Phase 6: Advanced Research
Exploration of next-generation retrieval paradigms.
*   **Goals:** Evaluate experimental architectures.
*   **Exit Criteria:**
    *   Successful evaluation of candidates listed in [RESEARCH.md](./RESEARCH.md) against defined quality, latency, memory, privacy, and energy budgets.
