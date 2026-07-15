# Architecture Research and Evaluation

**Date:** 2026-07-14
**Status:** NON-NORMATIVE

This document tracks experimental candidates for the Maestria search architecture. 
**Crucially, no model, backend, or specific algorithm listed here is a permanent default.** All candidates are treated as hypotheses.


## Legacy Report Status

`maestria_brioche_informed_code_architecture_report.md` is retained as historical
input and is **non-normative**. Its backend, model, and algorithm examples are
research candidates only. The canonical contracts are the documents listed in
[`SPECS.md`](SPECS.md) and the dated candidates in this document.

## 1. Evaluation Framework

Candidates are evaluated strictly against the Maestria internal corpora. A candidate is eligible for promotion to the [ROADMAP.md](./ROADMAP.md) only if it demonstrates superior performance across the following budgets:

*   **Quality:** Precision, recall, and relevance scores on standardized benchmarks.
*   **Latency:** P50, P90, and P99 response times for typical query loads.
*   **Memory:** Peak RAM and VRAM utilization during indexing and retrieval.
*   **Privacy:** Compliance with local-first processing requirements and data sovereignty guarantees.
*   **Security:** No ACL leakage, prohibited-candidate exposure, prompt-injection authorization, secret disclosure, or fail-open behavior.
*   **Energy:** Joules per query or indexing operation (critical for local/edge deployments).

## 2. Current Candidates (as of 2026-07-14)

### 2.1. Semantic Backends
*   **Candidate A (Local Embedding Model):** Evaluating sub-1B parameter models for entirely on-device semantic search.
    *   *Hypothesis:* Can achieve acceptable recall (within 10% of state-of-the-art) while keeping peak memory under 2GB.
*   **Candidate B (Sparse-Dense Hybrid):** Evaluating specific SPLADE or ColBERT variants.
    *   *Hypothesis:* Late interaction models provide better code-specific retrieval than dense-only models, fitting within a 500ms latency budget.

### 2.2. Reranking Strategies
*   **Candidate C (Cross-Encoder):** Evaluating small, distilled cross-encoders for the final reranking step.
    *   *Hypothesis:* Will improve Top-3 accuracy by 15% but may violate the 200ms latency budget; requires aggressive quantization.

## 3. Promotion Criteria

A candidate is promoted from this research document to an active architectural component only when:
1.  It conclusively beats the existing baseline in the Evaluation Framework.
2.  It satisfies all requirements of the [OPERATIONS.md](./OPERATIONS.md) (e.g., reproducibility, lifecycle management).
3.  The integration is abstracted, ensuring the candidate can be seamlessly replaced in the future.
