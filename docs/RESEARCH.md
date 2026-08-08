# Architecture Research and Evaluation

**Date:** 2026-07-20
**Status:** NON-NORMATIVE

This document tracks experimental candidates for the Maestria search architecture.
**Crucially, no model, backend, or specific algorithm listed here is a permanent default.** All candidates are treated as hypotheses.

## Legacy Report Status

`maestria_brioche_informed_code_architecture_report.md` is retained as historical
input and is **non-normative**. Its backend, model, and algorithm examples are
research candidates only. Canonical contracts are the documents listed in
[`SPECS.md`](SPECS.md); this document contains dated candidates only.

## 1. Evaluation Framework

Candidates are evaluated strictly against the Maestria internal corpora. A candidate is eligible for promotion to the [ROADMAP.md](./ROADMAP.md) only if it demonstrates superior performance across the following budgets:

* **Quality:** Precision, recall, exact-span retrieval, evidence-chain coverage, and relevance scores on standardized benchmarks.
* **Latency:** P50, P95, and P99 response times for typical query loads.
* **Memory:** Peak RAM and VRAM utilization during indexing and retrieval.
* **Storage and updates:** Index size, initial build cost, incremental update cost, deletion, rebuild, and rollback behavior.
* **Privacy:** Compliance with local-first processing requirements, data sovereignty, provider disclosure, and retention guarantees.
* **Security:** No ACL leakage, prohibited-candidate exposure, prompt-injection authorization, secret disclosure, quarantine escape, poisoning success, or fail-open behavior.
* **Energy:** Joules per query or indexing operation where the platform can measure them. Unavailable telemetry remains explicit and is never fabricated.

Synthetic or deterministic contract fixtures may prove schemas, lifecycle rules, and regressions. They do not prove product quality and cannot authorize a production promotion.

## 2. Current Candidates (as of 2026-07-20)

### 2.1. Learned-sparse candidate lane

Maestria defines learned sparse retrieval as an optional `sparse_text_v1` candidate lane. It is a distinct representation, not BM25 and not a dense-vector variant.

The experimental contract requires:

* a complete model, tokenizer, vocabulary, preprocessing, weighting, template, quantization, generation, and corpus identity;
* bounded positive term weights with duplicate and vocabulary-range rejection;
* pre-score scope, ACL, trust, sensitivity, quarantine, prompt-injection, current-version, and secret filtering;
* exact immutable evidence lineage;
* explicit sparse score and highest-contributing term provenance in search traces;
* independent provider and index adapters with shared contract tests;
* a dedicated execution policy that is `Shadow` by default, accepts only registry-validated shadow generations for non-serving experiments, records bounded serializable observations, and has no path back into served ranking, evidence, status, coverage, abstention, validation, or completion.

The deterministic in-memory provider and index are **contract fixtures only**. Their token hashing and weighting do not represent a trained learned-sparse model and must never be cited as retrieval-quality evidence.

A concrete learned-sparse route is eligible for activation only for a frozen query class when it:

1. beats the deterministic lexical baseline;
2. beats the currently eligible hybrid baseline;
3. preserves protected exact, no-evidence, abstention, and security behavior;
4. satisfies latency, memory, disk, indexing/update, privacy, security, and energy budgets with complete dated evidence; and
5. has a reversible promotion record bound to the exact corpus, judgments, model fingerprint, and index generation.

Removing or invalidating the promotion record restores the existing lexical/hybrid route. The presence of a provider or index adapter never activates sparse retrieval by itself.

### 2.1.0. Four-profile evaluation decision (dated 2026-08-08)

The frozen corpus (`tests/contracts/learned_sparse_task_corpus_v1.json`, revision
2026-07-30) was evaluated on a real instance against the pinned SPLADE ONNX sidecar
(`docs/RESEARCH.md` §4.3) across all four routes — Lexical, Hybrid, SparseOnly, and
SparseFused — with 31 timed runs per case per route, RAPL-measured energy, and the real
lifecycle operations measured on the durable projections. The candidate was re-pinned
on 2026-08-07 to the int8-quantized ONNX export with 512-token truncation (standard
SPLADE preprocessing), and the lane itself was optimized before the terminal run:
the projection gained an in-memory vector cache with term postings (searches
authorize and score only term-sharing chunks), the retriever gained per-request
artifact/evidence prefetch and per-artifact snapshot verification, and the sidecar
gained a parallel batch endpoint (bounded intra-op threads with a worker pool). The
dated report is `tests/contracts/learned_sparse_report_v1.json` (report hash
`sha256:c6a9017b5c1526dfe8403c09e537cfdc56551ab9fadb6b9e09219f505a668c88`), pinned
in the benchmark evidence ledger milestone `v1.2` with index generation
`sparse-text-v1-2` and the splade-onnx model fingerprint.

| Query class | Decision | Winning route | Rollback target |
| --- | --- | --- | --- |
| ExactLiteral | RetainLexical | none | — |
| VocabularyExpansion | RetainHybrid | none | — |
| DomainTerminology | RetainHybrid | none | — |
| MultiTerm | RetainHybrid | none | — |
| NoEvidence | RetainLexical | none | — |
| Security | RetainLexical | none | — |

No class won, on measured data, and the reasons are now precise:

- **Lifecycle passes**: initial indexing and rebuild of the 147-chunk corpus through
  the optimized lane measure ~3.2 s, under the 5 s `ingest_update_budget_ms` (down
  from ~17 s for the fp32 candidate and ~9 s for the first batch attempt).
- **Latency budget passes for every eligible class**: fused p95 mean is 191 ms
  (down from ~393 ms); the only two violations (274 ms, 303 ms) are ExactLiteral and
  Security cases, which are ineligible by design.
- **The promotion gate still refuses**, for two measured reasons. First, the gate's
  lifecycle-within-factor criterion compares the sparse lane's encode-based
  operations against the lexical route's tantivy projection (3.2 s vs 82 ms × 2):
  an HTTP-encode lane cannot meet a 2× factor over a local index at this corpus
  size, regardless of latency. Second, fused DomainTerminology quality regresses on
  MRR@10 (10.5 vs 11.3) even while recall@20 (35.7% vs 28.6%) and evidence-chain
  coverage (67.9% vs 60.7%) improve: the fusion re-ranks the first correct hit
  below the lexical baseline's.

The quality signal is real, reproducible, and survives the quantization and the
optimizations; exact, no-evidence, and security classes stay protected at zero across
routes. The lane remains benchmark-gated. The measured evidence says the remaining
blockers are a gate criterion that encode-based lanes cannot satisfy and a fusion
ranking regression — either a re-justified judgment set (documented budgets and
lifecycle factors) or a fusion change that preserves first-hit ranking would be the
next candidates for a new dated evaluation.

### 2.1.0a. Multilingual candidate evaluation (dated 2026-08-08)

The sparse lane is English-only today (`prithivida/Splade_PP_en_v1`, BERT
tokenizer). For multilingual contexts (e.g. French users), BGE-M3 was
investigated as the leading candidate (MIT license, 100+ languages, explicit
sparse output). Verdict: **not evaluable and not budget-feasible**:

- **The trained sparse head is not released.** The official `BAAI/bge-m3`
  checkpoint contains only the backbone (391 keys); the published
  `sparse_linear.pt` is a 3.5 KB stub holding a `[1, 1024]` tensor, not the
  trained `[250002, 1024]` projection. FlagEmbedding's own loader falls back
  to a *randomly initialized* head when the `.pt` is missing or unusable, so
  even the reference library cannot serve the real sparse model from the
  release. Community ONNX exports either omit the head or export a broken
  `sparse_vecs` output (vocab dimension collapsed to 1, verified in
  `aapot/bge-m3-onnx`).
- **Throughput excludes it from the frozen budgets.** Measured on this
  machine (2-thread ONNX session): fp32 2.13 s and int8 0.79 s per 512-token
  encode; the 147-chunk corpus re-encode with 6 parallel workers measures
  164.6 s fp32 and 65.8 s int8 — 13× over the 5 s `ingest_update_budget_ms`
  and ~15× over the gate's lifecycle-within-factor allowance against the
  lexical route.

The lexical lane itself is language-agnostic (tantivy's Unicode-aware default
tokenizer: no stemming, no stopword language, accents preserved), so a
non-English user already gets working exact-term retrieval today; the sparse
lane being unpromoted means no English-model noise is injected. The principled
multilingual path is therefore: (1) extend the frozen corpus with non-English
cases and judged spans (the corpus format has no language restriction), and
(2) evaluate a budget-fitting candidate — either a multilingual sparse
checkpoint of SPLADE-class size (XLM-R-base-class, ~50-150 ms/text int8) when
one with a clean license appears, or BGE-M3's *dense* output through the
dense lane (the released, trained, multilingual artifact) under a
re-justified judgment set (issue #427).

### 2.1.1. Frozen learned-sparse task corpus

The representative real-task freeze is `tests/contracts/learned_sparse_task_corpus_v1.json`.
Its source manifest is content-addressed and names repository-relative task and evidence inputs.
Normal retrieval cases use real Maestria task identifiers; synthetic cases are limited to
adversarial and lifecycle coverage. Each final query class has two independent task cases,
while development cases remain separate from the frozen final split.

Judgments use an explicit three-level relevance scale, accepted exact spans, evidence-chain
identities, citation expectations, freshness requirements, and security outcomes. Two judges
work independently; disagreement is adjudicated by a third judge. The corpus validator rejects
unknown sources, duplicate cases, path traversal, missing split coverage, underrepresented
final classes, and incomplete expectations. Changing source content, judgment guidance, or
judgments requires new corpus and judgment hashes.

Note: the frozen final split (two independent task cases per class) is the dated judgment
set as authored at the 2026-07-30 freeze; it has not been re-tuned or re-weighted for this
evaluation. The 2026-08-07 decision above is bound to exactly this split and hash; any
split change requires a new evaluation and a new dated decision.

### 2.2. Other semantic backends

* **Candidate A (Local Embedding Model):** Evaluating sub-1B parameter models for entirely on-device semantic search.
  * *Hypothesis:* Can achieve acceptable recall while keeping peak memory compatible with consumer machines.
* **Candidate B (Sparse-Dense Hybrid):** Evaluating contract-compatible sparse and dense combinations after each independent lane has valid benchmark evidence.
  * *Hypothesis:* Sparse vocabulary expansion may complement dense semantic recall for selected query classes without weakening exact retrieval.
* **Candidate C (Late Interaction):** Evaluating bounded multi-vector reranking before considering a dedicated index.
  * *Hypothesis:* Fine-grained token interaction may improve code and long-document matching, but storage and compute costs may prevent first-stage indexing.

### 2.3. Reranking strategies

* **Candidate D (Cross-Encoder):** Evaluating small, distilled cross-encoders for the final reranking step.
  * *Hypothesis:* A bounded reranker can improve final evidence ordering, but only where its latency and privacy costs fit the plan budget.

## 4. Pinned local sidecar profiles (dated candidates, 2026-07-31)

The profiles below are dated implementation candidates for the optional OCR
and visual capabilities. They are not normative: revisions, artifact hashes,
and endpoints change with evaluation. The README keeps only the agnostic
capability boundary; re-pinning a profile updates this section.

### 4.1. OCR profile: RapidOCR sidecar (CPU, ONNX Runtime)

```bash
uv venv .venv-rapidocr
uv pip install --python .venv-rapidocr/bin/python \
  -r scripts/requirements-rapidocr.txt
.venv-rapidocr/bin/python scripts/rapidocr_server.py \
  --host 127.0.0.1 --port 10000
```

Manifest keys:

```text
ocr_enabled=true
ocr_endpoint=http://127.0.0.1:10000/v1/chat/completions
ocr_provider=rapidai
ocr_revision=rapidocr-onnxruntime-1.4.4
ocr_artifact_hash=sha256:971d7d5f223a7a808662229df1ef69893809d8457d834e6373d3854bc1782cbf
ocr_preprocessing_version=pdf-pdftoppm-v1
ocr_model=rapidocr-onnxruntime-1.4.4
```

The adapter renders only pages requiring OCR with the local `pdftoppm` binary
and sends image bytes to the sidecar over the loopback OpenAI-compatible
contract. CPU-capable ONNX Runtime inference; Maestria never downloads or
executes model code.

### 4.2. Visual profile: SigLIP ONNX (CPU) with optional Qwen3-VL-Embedding

```bash
uv venv .venv-visual
uv pip install --python .venv-visual/bin/python \
  -r scripts/requirements-visual.txt
```

Download the pinned SigLIP artifacts from `Xenova/siglip-base-patch16-224` at
revision `4649052661e53c7000355844105f8a1792088239`, then start the sidecar
with the quantized ONNX artifacts:

```bash
.venv-visual/bin/python scripts/siglip_visual_server.py \
  --host 127.0.0.1 --port 10001 \
  --model siglip-base-patch16-224-int8 \
  --vision-model .maestria/models/siglip/onnx/vision_model_int8.onnx \
  --text-model .maestria/models/siglip/onnx/text_model_int8.onnx \
  --tokenizer .maestria/models/siglip/tokenizer.json
```

Compute the artifact fingerprint before enabling the profile:

```bash
python3 scripts/visual_model_fingerprint.py \
  --profile siglip_cpu \
  --model-dir .maestria/models/siglip
```

Manifest keys (set `visual_artifact_hash` to the fingerprint output):

```text
visual_enabled=true
visual_endpoint=http://127.0.0.1:10001/v1/embeddings
visual_provider=siglip-onnx
visual_revision=4649052661e53c7000355844105f8a1792088239
visual_artifact_hash=sha256:<fingerprint-output>
visual_preprocessing_version=siglip-224-rgb-v1
visual_model=siglip-base-patch16-224
visual_dimensions=768
visual_remote_provider=false
visual_retention_policy=no_retention
```

Both sidecars accept loopback traffic only, perform CPU inference, and retain
no inputs. Visual activation additionally requires a matching fingerprinted
`visual_page_v1` generation and a passing benchmark; otherwise the app keeps
the text/layout route.

### 4.3. Learned-sparse profile: SPLADE ONNX (CPU)

Dated research candidate (pinned 2026-08-07). The checkpoint is the
SPLADE++/cocondenser-family export `prithivida/Splade_PP_en_v1` (revision
`762be6a7206e2f299182705972a65e5c46e62be2`, Apache-2.0, distilbert vocabulary
of 30 522 terms). Re-pinning a different SPLADE-family checkpoint updates this
section and the sparse manifest keys.

```bash
uv venv .venv-sparse
uv pip install --python .venv-sparse/bin/python \
  -r scripts/requirements-sparse.txt
```

Download the pinned artifacts into `.maestria/models/splade/onnx/` (from
`prithivida/Splade_PP_en_v1` at the revision above), then start the sidecar:

```bash
.venv-sparse/bin/python scripts/splade_server.py \
  --host 127.0.0.1 --port 10002 \
  --model prithivida/Splade_PP_en_v1 \
  --model-dir .maestria/models/splade
```

Compute the artifact fingerprint before enabling the profile:

```bash
python3 scripts/sparse_model_fingerprint.py \
  --profile splade_pp_en_v1_cpu \
  --model-dir .maestria/models/splade
```

Manifest keys (set `sparse_artifact_hash` to the fingerprint output):

```text
sparse_enabled=true
sparse_endpoint=http://127.0.0.1:10002/v1/sparse
sparse_provider=splade-onnx
sparse_revision=762be6a7206e2f299182705972a65e5c46e62be2
sparse_artifact_hash=sha256:<fingerprint-output>
# pinned 2026-08-07 re-pin: int8-quantized ONNX (2.3x encode speedup) with
# 512-token truncation (standard SPLADE preprocessing); the fp32 checkpoint
# remains available as onnx/model_fp32.onnx outside the pinned artifact set
sparse_preprocessing_version=splade-templates-trunc512-v1
sparse_model=prithivida/Splade_PP_en_v1
sparse_vocabulary_size=30522
sparse_term_cap=256
sparse_remote_provider=false
sparse_retention_policy=no_retention
```

The sidecar applies the `query: {text}` / `document: {text}` templates, caps
term vectors at 256 terms, accepts loopback traffic only, performs CPU
inference, and retains no inputs. It also serves `POST /v1/sparse/batch`
(`{"texts": [...], "kind": "document"}` → one vector per text, input order
preserved) with a bounded worker pool over a two-thread ONNX session, which is
what the daemon's projection reconciliation and the benchmark lifecycle
operations use. Unlike the embedding/visual profiles, a
remote provider or retained retention policy is a manifest error, not a
deferred rejection. Sparse activation additionally requires a matching
fingerprinted `sparse_text_v1` generation and a passing benchmark; otherwise
the app keeps the lexical/hybrid route.

## 3. Promotion Criteria

A candidate is promoted from this research document to an active architectural component only when:

1. It conclusively beats every required existing baseline on a frozen, versioned Maestria evaluation corpus.
2. It satisfies all requirements of [OPERATIONS.md](./OPERATIONS.md), including reproducibility, generation lifecycle, cancellation, degradation, and rollback.
3. The integration is abstracted behind provider-neutral contracts and remains replaceable.
4. The dated report records corpus, judgment, model/index, environment, quality, resource, privacy, security, and energy evidence.
5. Promotion is restricted to the query classes and exact route configuration that won; all other paths remain shadowed or use the conservative baseline.
