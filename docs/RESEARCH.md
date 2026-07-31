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

## 3. Promotion Criteria

A candidate is promoted from this research document to an active architectural component only when:

1. It conclusively beats every required existing baseline on a frozen, versioned Maestria evaluation corpus.
2. It satisfies all requirements of [OPERATIONS.md](./OPERATIONS.md), including reproducibility, generation lifecycle, cancellation, degradation, and rollback.
3. The integration is abstracted behind provider-neutral contracts and remains replaceable.
4. The dated report records corpus, judgment, model/index, environment, quality, resource, privacy, security, and energy evidence.
5. Promotion is restricted to the query classes and exact route configuration that won; all other paths remain shadowed or use the conservative baseline.
