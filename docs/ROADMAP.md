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
    *   Routing and decomposition are evaluated against a versioned judgment set and never bypass scope, ACL, trust, sensitivity, quarantine, or prompt-injection filters before scoring or exposure.
    *   Every plan has explicit budgets, stopping conditions, evidence coverage, and abstention behavior.
    *   Implementation of the observation/candidate/promotion lifecycle detailed in [MEMORY.md](./MEMORY.md).

## Phase 4: Code Intelligence
Deep structural and contextual understanding of codebases.
*   **Goals:** Cross-file dependency resolution, type-aware search, and semantic similarity of code blocks.
*   **Exit Criteria:**
    *   A versioned Rust repository benchmark reports symbol, relation, and exact-span recall for supported language features across frozen query classes.
    *   Live freshness checks detect changed worktree sources without returning stale evidence as current.
    *   Code retrieval preserves ACL, provenance, and deterministic trace requirements.
    *   Specialized routing is shadowed by default and activates only for query classes with a measured evidence-quality, freshness, and latency win; unsupported or unsafe questions abstain.
    *   The phase uses the shared Rule 44 evaluation contract: quality, latency, memory, privacy, security, and energy are measured on the versioned corpus.

## Phase 5: Visual Documents
Expand search capabilities to include visual assets and structured document regions.
*   **Goals:** Retrieve pages, tables, charts, figures, and visual regions with source coordinates.
*   **Exit Criteria:**
    *   A versioned visual-document benchmark reports region recall, citation alignment, and table/figure coverage.
    *   Visual candidates retain immutable page/region provenance and pass the same scope, trust, and quarantine gates as text.
    *   The phase uses the shared Rule 44 evaluation contract: quality, latency, memory, privacy, security, and energy are measured on the versioned corpus.

## Phase 6: Advanced Research
Explore additional retrieval paradigms only where measured quality/cost/security frontiers justify them.
*   **Goals:** Evaluate experimental architectures without turning candidates into permanent defaults.
*   **Exit Criteria:**
    *   Candidates listed in [RESEARCH.md](./RESEARCH.md) are evaluated against quality, latency, memory, privacy, security, and energy budgets.
    *   Promotion records include reproducible corpus/index fingerprints, rollback compatibility, and a dated decision.
