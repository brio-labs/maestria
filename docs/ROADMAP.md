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


## Version-to-Phase Mapping

The following table maps packaged releases to the roadmap phases they address
and marks each capability surface with its current status.

| Capability | Phase | Status | Shipped In |
|---|---|---|---|
| Local file indexing | 1 | Stable | v0.1-v0.5 |
| Lexical (Tantivy) search | 1 | Stable | v0.5 |
| Evidence opening | 1 | Stable | v0.5 |
| Daemon projection | 1 | Stable | v0.6 |
| Task lifecycle & validation | 1 | Stable | v0.6 |
| Approval resolution | 1 | Stable | v0.6 |
| Memory candidates & promotion | 1 | Stable | v0.6 |
| Search observability (explain/trace/compare) | 3 | Stable | v0.6 |
| Index generations observability | 3 | Stable | v0.6 |
| Evidence coverage observability | 3 | Stable | v0.6 |
| Repository/Cargo code indexing | 4 | Shadowed | v0.6 |
| Code symbol/path/regex/context search | 4 | Shadowed | v0.6 |
| Visual document region retrieval | 5 | Research-only | — |
| Dense (embedding) retrieval | 2 | Provider-dependent | v0.6 |
| Web evidence with governed adapter | 3 | Provider-dependent | v0.6 |
| Advanced dense / learned-sparse reranking | 2 | Research-only | — |
| Late-interaction / graph / temporal retrieval | 6 | Research-only | — |
| Multimodal promotion | 6 | Research-only | — |

### Status definitions

| Status | Meaning |
|---|---|
| **Stable** | Shipped in the current binary, tested, and supported. Breaking changes require a major version bump and documented migration. |
| **Shadowed** | Implemented but inactive by default. Activates only when a measured benchmark proves a quality and resource win for a query class (Phase 4 rule). |
| **Provider-dependent** | Present in the binary but requires an external provider (web adapter, embedding service). Degrades explicitly when the provider is unavailable. |
| **Research-only** | Implemented in evaluation branches or experimental crates. Not shipped in release binaries. See [`RESEARCH.md`](./RESEARCH.md). |

### Current release versus workspace version

The workspace version in `Cargo.toml` (`0.6.1` at the time of writing) is the
next planned binary version and is independent of any previously published
release. A release consumes the current workspace version only after passing
all exit-evidence gates. The latest published release is `v0.6.1`.

## Release Milestone Exit Stages

Operational progress is now tracked through explicit milestone exit evidence, and milestone
close/readiness is no longer a binary signal.

Release milestones must publish a machine-readable exit-evidence block in the milestone
description before they can be used for release publication.

The block must encode one of:

*   **planned** — the milestone is specified but one or more implementation issues remain open;
*   **implementation-complete** — all implementation issues are closed and tracked;
*   **benchmark-complete** — benchmark measurements are collected and linked;
*   **product-complete** — benchmark measurements include version-coupled fingerprints,
  quality/resource/security results, and degradations with real measurements where possible;
*   **released** — product completion has been published and `post_release_work` is populated.

### Synthetic-vs-Real Distinction

`benchmark.data_fidelity` must explicitly document whether measurements were run on
synthetic data or a real benchmark corpus. The allowed values are:

*   **real** — measurements on a live/production benchmark corpus.
*   **synthetic** — measurements on synthetic or generated data.
*   **mixed** — combination of real and synthetic data.
*   **staged** — measurements on a staging/pre-production corpus.

Synthetic or staged measurements are sufficient for `benchmark-complete` but cannot
certify `product-complete` or `released`.

### Benchmark Environment and Artifacts

For reproducibility, evidence blocks SHOULD include:

*   `benchmark.environment` — the benchmark runtime environment:
    *   `os` — operating system identifier (e.g. `ubuntu-24.04`).
    *   `rust_toolchain` — Rust toolchain (e.g. `stable`, `nightly-2026-06-15`).
    *   `cpu_arch` — CPU architecture (e.g. `x86_64`, `aarch64`).
*   `benchmark.artifacts` — links to CI runs or external reports:
    *   `source` — one of `ci`, `manual`, `external`.
    *   `url` — URL to the artifact.
    *   `label` — human-readable label.

### GoldenProfile Support

Evidence blocks MAY include a `profiles` section for tracking benchmark profiles
across release cycles:

*   `profiles.version` — profile schema version (currently `1`).
*   `profiles.entries` — array of profile entries, each with:
    *   `stage` — one of `baseline`, `golden`, `shadow`, `promoted`, `retired`.
    *   `name` — unique profile name.

### Exit Evidence Contract (Minimum Fields)

At minimum, the evidence block must include:

*   `release_stage`
*   `schema_version` (current value: `1`)
*   `benchmark.benchmark_date`
*   `benchmark.fingerprints`:
    *   `corpus_snapshot`
    *   `index_generation`
    *   `model_fingerprint`
*   `benchmark.results.quality`
*   `benchmark.results.resource`
*   `benchmark.results.security`
*   `benchmark.degradations`

Post-release follow-up entries should target the repository maintenance/release grouping
when it exists, with at least one entry for every follow-up stream.

### Example: Product-Complete Evidence Block

```markdown
```release-exit-evidence
{
  "schema_version": 1,
  "release_stage": "product-complete",
  "benchmark": {
    "benchmark_date": "2026-07-19",
    "data_fidelity": "real",
    "fingerprints": {
      "corpus_snapshot": "corpus-v1",
      "index_generation": "idx-42",
      "model_fingerprint": "provider:rerank-v3"
    },
    "environment": {
      "os": "ubuntu-24.04",
      "rust_toolchain": "stable",
      "cpu_arch": "x86_64"
    },
    "artifacts": [
      {
        "source": "ci",
        "url": "https://github.com/example/maestria/actions/runs/12345",
        "label": "Benchmark CI run #12345"
      }
    ],
    "results": {
      "quality": {"status": "pass", "p50": 0.74, "p95": 0.88},
      "resource": {"status": "pass", "p95_latency_ms": 120, "memory_mb": 256},
      "security": {"status": "pass", "violations": 0}
    },
    "degradations": [
      {
        "area": "query_class",
        "status": "known",
        "description": "Table evidence is incomplete on scanned PDFs"
      }
    ]
  },
  "post_release_work": []
}
```
```

### Example: Benchmark-Complete with Staged Data and Profiles

```markdown
```release-exit-evidence
{
  "schema_version": 1,
  "release_stage": "benchmark-complete",
  "benchmark": {
    "benchmark_date": "2026-07-20",
    "data_fidelity": "staged",
    "fingerprints": {
      "corpus_snapshot": "corpus-v1-staging",
      "index_generation": "idx-42",
      "model_fingerprint": "provider:rerank-v3"
    },
    "results": {
      "quality": {"status": "pass", "p50": 0.70},
      "resource": {"status": "pass", "p95_latency_ms": 130},
      "security": {"status": "pass", "violations": 0}
    },
    "degradations": []
  },
  "profiles": {
    "version": 1,
    "entries": [
      {"stage": "baseline", "name": "corpus-v1-baseline"},
      {"stage": "golden", "name": "corpus-v1-golden"}
    ]
  },
  "post_release_work": [
    {
      "group": "maintenance/release",
      "status": "open",
      "description": "Run production benchmark with real corpus data"
    }
  ]
}
```
```

### CLI Usage

The `scripts/release_exit_evidence.py` tool exposes subcommands:

```text
# Validate milestone exit evidence
python3 scripts/release_exit_evidence.py validate \
  --description-file milestone_description.md \
  --required-stage product-complete \
  --require-maintenance-grouping \
  --milestone-title "v0.7"

# Generate an exit evidence block
python3 scripts/release_exit_evidence.py generate \
  --release-stage benchmark-complete \
  --benchmark-date 2026-07-20 \
  --data-fidelity real \
  --corpus-snapshot corpus-v1 \
  --index-generation idx-42 \
  --model-fingerprint provider:rerank-v3 \
  --environment-os ubuntu-24.04 \
  --environment-rust-toolchain stable \
  --environment-cpu-arch x86_64

# Reconcile exit evidence against actual metrics
python3 scripts/release_exit_evidence.py reconcile \
  --evidence-file milestone_description.md \
  --actual-results-file actual_results.json \
  --actual-environment-file actual_environment.json

# Validate post-release tracking completeness
python3 scripts/release_exit_evidence.py validate-tracking \
  --work-items-file post_release_work.json \
  --follow-up-issues-file follow_up_issues.json
```

## Historical Milestone Evidence

The following table records the verified release exit stage for each published
and planned release milestone. Milestones marked **Closable** have sufficient
evidence recorded in this repository to close; milestones marked **Open** are
pre-populated stubs that require benchmark evidence before advancing beyond
`implementation-complete`.

Every milestone whose closure is proposed MUST include a fenced
`release-exit-evidence` block in its GitHub description. The canonical text
for each stage is given below and in the machine-readable manifest at
`tests/contracts/milestone_evidence_v0.4_v0.9.json`. The lead applies the
canonical block to the GitHub milestone description after merge, then closes
only milestones whose `release_stage` is not capped below the required stage.

| Milestone | Release Stage | Data Fidelity | Summary | Closure |
|---|---|---|---|---|
| v0.4 — Deterministic Search Baseline | implementation-complete | — | Local file indexing; no benchmark corpus recorded | Historical closed |
| v0.5 — Evaluated Hybrid Retrieval | implementation-complete | — | Lexical search and evidence opening; no benchmark corpus recorded | Historical closed |
| v0.6 — Query-Adaptive Search | implementation-complete | — | Query-adaptive search, code intelligence, memory promotion (v0.6.1 latest); no quality/resource/security measurements recorded | Historical closed |
| v0.7 — Repository Intelligence | implementation-complete | — | Implementation issues closed; benchmark evidence still required | Open |
| v0.8 — Visual Document Retrieval | implementation-complete | — | Implementation issues closed; benchmark evidence still required | Open |
| v0.9 — Advanced Retrieval Research | planned | — | Research issues #90–#95 remain open; benchmark-gated research is planned | Open |

### Canonical Milestone Description

For a `planned` milestone, set the GitHub description to:

```markdown
```release-exit-evidence
{
  "schema_version": 1,
  "release_stage": "planned"
}
```
```

For an `implementation-complete` milestone, set the same block with
`"release_stage": "implementation-complete"`.

For a `benchmark-complete` milestone with staged measurements:

```markdown
```release-exit-evidence
{
  "schema_version": 1,
  "release_stage": "benchmark-complete",
  "benchmark": {
    "benchmark_date": "<YYYY-MM-DD>",
    "data_fidelity": "staged",
    "fingerprints": {
      "corpus_snapshot": "<corpus-id>",
      "index_generation": "<index-id>",
      "model_fingerprint": "<provider:model>"
    },
    "results": {
      "quality": {"status": "pass"},
      "resource": {"status": "pass"},
      "security": {"status": "pass", "violations": 0}
    },
    "degradations": []
  },
  "post_release_work": [
    {
      "group": "maintenance/release",
      "status": "open",
      "description": "Run production benchmark with real corpus data"
    }
  ]
}
```
```

The `benchmark-complete` post-release work entry SHALL target the
`maintenance/release` grouping when the repository maintenance/release grouping
exists (it does as of v0.6.1). This satisfies the `require-maintenance-grouping`
flag used by the release workflow preflight gate.

### Applying the Evidence Block

After this PR merges, the lead MUST:
1. Read the per-milestone evidence from the machine-readable manifest at
   `tests/contracts/milestone_evidence_v0.4_v0.9.json`.
2. Apply the manifest's exact evidence block to every actual GitHub milestone:
   `v0.4 — Deterministic Search Baseline`, `v0.5 — Evaluated Hybrid Retrieval`,
   `v0.6 — Query-Adaptive Search`, `v0.7 — Repository Intelligence`,
   `v0.8 — Visual Document Retrieval`, and
   `v0.9 — Advanced Retrieval Research`.
3. Preserve the already-closed historical state of v0.4, v0.5, and v0.6, but
   do not describe them as benchmark-complete or product-complete.
4. Leave v0.7 and v0.8 open until benchmark issues
   [#85](https://github.com/brio-labs/maestria/issues/85) and
   [#89](https://github.com/brio-labs/maestria/issues/89) provide their
   versioned evidence.
5. Leave v0.9 open and planned while research issues
   [#90](https://github.com/brio-labs/maestria/issues/90)–[#95](https://github.com/brio-labs/maestria/issues/95)
   remain open; those issues are the explicit follow-up assignments.
6. Validate every milestone description before changing its state:
   ```text
   python3 scripts/release_exit_evidence.py validate \
     --description-file <(gh api ... -q '.description') \
     --required-stage planned
   ```

All stub and historical evidence payloads are checked-in and CI-validated by
`scripts/test_milestone_evidence.py` against the release-exit-evidence contract.
