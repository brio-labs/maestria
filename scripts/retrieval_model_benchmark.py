#!/usr/bin/env python3
"""Cross-model retrieval quality benchmark (BEIR/MTEB-style methodology).

Compares retrieval models on standard datasets with standard metrics:

- Datasets: MS MARCO passage dev (English) and mMARCO dev (French), served
  by ir_datasets (the loader behind BEIR). Passages and queries are sampled
  deterministically (seeded) to fit a laptop CPU; the sample is documented
  with the results.
- Metrics (BEIR conventions, binary gains): nDCG@10, MRR@10, Recall@10,
  Recall@100.
- Encoding conventions follow each model's reference usage: SPLADE
  query/document templates, e5 "query:"/"passage:" prefixes, mLateOn
  [Q]/[D] prefix tokens and MaxSim late interaction, BGE-M3 CLS pooling,
  LFM2.5 CLS pooling, MiniLM mean pooling, BM25 reference (k1=1.2, b=0.75,
  matching tantivy's defaults).
- Vectors are cached under .maestria/bench-vectors so re-runs skip encodes.

Usage:
    python3 scripts/retrieval_model_benchmark.py [--langs en fr] \
        [--queries 200] [--passages 5000] [--seed 42] [--models ...]

The report is written to target/benchmark-reports/model-retrieval.json.
"""

from __future__ import annotations

import argparse
import json
import math
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
MODELS_DIR = ROOT / ".maestria" / "models"
CACHE_DIR = ROOT / ".maestria" / "bench-vectors"
REPORT_DIR = ROOT / "target" / "benchmark-reports"

MAX_TOKENS = 512
BATCH_WORKERS = 6
CHUNK = 64

# ---------------------------------------------------------------------------
# Data loading (ir_datasets, the BEIR backend)
# ---------------------------------------------------------------------------


def load_dataset(lang: str, rng, n_queries: int, n_passages: int):
    """Loads queries, a judged passage corpus, and qrels.

    The corpus sample keeps every judged relevant passage of the sampled
    queries and fills the rest with seeded random passages; a purely random
    sample would drop the qrel documents and make every metric zero.
    The sampled dataset is cached on disk: the full-corpus scan that builds
    it costs 6-8 minutes for mMARCO-fr and never changes for fixed
    parameters.
    """
    import pickle

    cache = CACHE_DIR / f"dataset_{lang}_c{n_passages}_q{n_queries}_s{args.seed}.pkl"
    if cache.exists():
        with open(cache, "rb") as handle:
            return pickle.load(handle)
    import ir_datasets

    dataset_id = "msmarco-passage/dev" if lang == "en" else "mmarco/fr/dev"
    dataset = ir_datasets.load(dataset_id)
    queries = list(dataset.queries_iter())
    rng.shuffle(queries)
    queries = queries[:n_queries]
    qids = {q.query_id for q in queries}
    all_qrels = list(dataset.qrels_iter())
    qrels_for = [qrel for qrel in all_qrels if qrel.query_id in qids]
    relevant_ids = {qrel.doc_id for qrel in qrels_for}

    # Pass 1: the judged relevant passages.
    relevant = {}
    for doc in dataset.docs_iter():
        if doc.doc_id in relevant_ids:
            relevant[doc.doc_id] = doc.text

    # Pass 2: reservoir sample of non-relevant fillers.
    fillers = []
    capacity = max(n_passages - len(relevant), 0)
    seen = 0
    for doc in dataset.docs_iter():
        if doc.doc_id in relevant_ids:
            continue
        seen += 1
        if len(fillers) < capacity:
            fillers.append(doc)
        else:
            j = rng.integers(0, seen)
            if j < capacity:
                fillers[j] = doc
    rng.shuffle(fillers)
    passages = [
        type("Doc", (), {"doc_id": doc_id, "text": text})()
        for doc_id, text in relevant.items()
    ] + fillers[:capacity]
    passages = [type("Doc", (), {"doc_id": doc.doc_id, "text": doc.text})() for doc in passages]
    rng.shuffle(passages)
    passage_ids = {doc.doc_id for doc in passages}
    qrels = {
        (qrel.query_id, qrel.doc_id)
        for qrel in qrels_for
        if qrel.doc_id in passage_ids
    }
    cache.parent.mkdir(parents=True, exist_ok=True)
    with open(cache, "wb") as handle:
        pickle.dump((queries, passages, qrels), handle)
    return queries, passages, qrels


# ---------------------------------------------------------------------------
# Encoders.  Each encoder maps (texts: list[str], is_query: bool) to vectors:
#   dense  -> np.ndarray [n, dim]
#   sparse -> list of (np.ndarray terms, np.ndarray weights)
#   late   -> list of np.ndarray [seq, dim] token matrices
# ---------------------------------------------------------------------------


class TokenizerWrapper:
    def __init__(self, path: str):
        from tokenizers import Tokenizer

        self._tok = Tokenizer.from_file(path)
        self._pad_id = self._tok.token_to_id("[PAD]")

    def encode(self, text: str, max_tokens: int, prefix_id: int | None = None):
        enc = self._tok.encode(text)
        enc.truncate(max_tokens - (1 if prefix_id is not None else 0))
        ids = [prefix_id] + enc.ids if prefix_id is not None else enc.ids
        # sentence-transformers ONNX tokenizers pad to a fixed length and
        # count the pad tokens as real in the attention mask; zero the mask
        # at pad positions so pooling ignores them.
        if self._pad_id is not None:
            mask = [0 if token == self._pad_id else 1 for token in ids]
        else:
            mask = [1] * len(ids)
        return np.asarray([ids], dtype=np.int64), np.asarray([mask], dtype=np.int64)


class OnnxSession:
    def __init__(self, model_path: str, threads: int = 2):
        import onnxruntime as ort

        opts = ort.SessionOptions()
        opts.intra_op_num_threads = threads
        self._sess = ort.InferenceSession(
            model_path, sess_options=opts, providers=["CPUExecutionProvider"]
        )
        self._names = {i.name for i in self._sess.get_inputs()}

    def run(self, ids: np.ndarray, mask: np.ndarray) -> np.ndarray:
        feed = {"input_ids": ids}
        if "input_mask" in self._names:
            feed["input_mask"] = mask
        elif "attention_mask" in self._names:
            feed["attention_mask"] = mask
        if "segment_ids" in self._names:
            feed["segment_ids"] = np.zeros_like(ids)
        elif "token_type_ids" in self._names:
            feed["token_type_ids"] = np.zeros_like(ids)
        return self._sess.run(None, feed)[0]


def l2(v):
    return v / np.linalg.norm(v)


def make_encoder(name: str):
    mdir = MODELS_DIR

    if name == "bm25":
        def bm25(texts, is_query):
            return bm25_vectors(texts)
        return bm25, "sparse"

    if name == "splade":
        tok = TokenizerWrapper(str(mdir / "splade" / "tokenizer.json"))
        sess = OnnxSession(str(mdir / "splade" / "onnx" / "model.onnx"))

        def splade(texts, is_query):
            template = "query: {text}" if is_query else "document: {text}"
            vectors = []
            for text in texts:
                ids, mask = tok.encode(template.format(text=text), MAX_TOKENS)
                logits = np.asarray(sess.run(ids, mask)[0][0], dtype=np.float32)
                if logits.ndim == 2:
                    logits = logits.max(axis=0)
                logits = logits.reshape(-1)
                weights = np.log1p(np.maximum(logits, 0.0))
                order = np.argsort(-weights)[:256]
                order = order[weights[order] > 0.0]
                vectors.append((order.astype(np.int64), weights[order].astype(np.float64)))
            return vectors
        return splade, "sparse"

    def mean_pool_dense(model_path, tokenizer_path, max_tokens=MAX_TOKENS):
        tok = TokenizerWrapper(tokenizer_path)
        sess = OnnxSession(model_path)

        def encode(texts, is_query):
            out = np.empty((len(texts), 0), dtype=np.float32)
            results = []
            for text in texts:
                ids, mask = tok.encode(text, max_tokens)
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                m = mask.astype(np.float32)
                pooled = (hidden * m[:, :, None]).sum(axis=1) / m.sum(axis=1, keepdims=True)
                results.append(l2(pooled[0]).astype(np.float32))
            return np.stack(results)
        return encode

    def cls_pool_dense(model_path, tokenizer_path, max_tokens=MAX_TOKENS, prefix=None):
        tok = TokenizerWrapper(tokenizer_path)
        sess = OnnxSession(model_path)

        def encode(texts, is_query):
            results = []
            for text in texts:
                ids, mask = tok.encode(text, max_tokens)
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                pooled = hidden[0][0] if hidden.ndim == 3 else hidden[0]
                results.append(l2(pooled).astype(np.float32))
            return np.stack(results)
        return encode

    if name == "minilm":
        return mean_pool_dense(
            str(mdir / "minilm-l6" / "onnx" / "model_quint8_avx2.onnx"),
            str(mdir / "minilm-l6" / "tokenizer.json"),
            max_tokens=256,
        ), "dense"

    if name == "e5-small":
        tok = TokenizerWrapper(str(mdir / "e5-small" / "tokenizer.json"))
        sess = OnnxSession(str(mdir / "e5-small" / "onnx" / "model_int8.onnx"))

        def e5(texts, is_query):
            prefix = "query: " if is_query else "passage: "
            results = []
            for text in texts:
                ids, mask = tok.encode(prefix + text, MAX_TOKENS)
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                m = mask.astype(np.float32)
                pooled = (hidden * m[:, :, None]).sum(axis=1) / m.sum(axis=1, keepdims=True)
                results.append(l2(pooled[0]).astype(np.float32))
            return np.stack(results)
        return e5, "dense"

    if name == "bge-m3":
        return cls_pool_dense(
            str(mdir / "bge-m3" / "onnx" / "model_int8.onnx"),
            str(mdir / "bge-m3" / "onnx" / "tokenizer.json"),
        ), "dense"

    if name == "lfm25":
        tok = TokenizerWrapper(str(mdir / "lfm25-embedding" / "tokenizer.json"))
        # int8 re-quantized from the corrected bidirectional export (cos
        # 0.97-0.99 vs fp32); the first export was broken by a wrong model
        # class, not by quantization.
        sess = OnnxSession(str(mdir / "lfm25-embedding" / "onnx" / "model_int8.onnx"))

        def lfm25(texts, is_query):
            prefix = "query: " if is_query else "document: "
            results = []
            for text in texts:
                ids, mask = tok.encode(prefix + text, MAX_TOKENS)
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                pooled = hidden[0][0] if hidden.ndim == 3 else hidden[0]
                results.append(l2(pooled).astype(np.float32))
            return np.stack(results)
        return lfm25, "dense"

    if name == "minilm-l12":
        return mean_pool_dense(
            str(mdir / "minilm-l12" / "onnx" / "model.onnx"),
            str(mdir / "minilm-l12" / "tokenizer.json"),
            max_tokens=256,
        ), "dense"

    if name == "mdenseon":
        tok = TokenizerWrapper(str(mdir / "mdenseon" / "tokenizer.json"))
        sess = OnnxSession(str(mdir / "mdenseon" / "onnx" / "model_int8.onnx"))

        def mdenseon(texts, is_query):
            prefix = "query: " if is_query else "document: "
            results = []
            for text in texts:
                ids, mask = tok.encode(prefix + text, MAX_TOKENS)
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                pooled = hidden[0][0] if hidden.ndim == 3 else hidden[0]
                results.append(l2(pooled).astype(np.float32))
            return np.stack(results)
        return mdenseon, "dense"

    if name == "nomic":
        tok = TokenizerWrapper(str(mdir / "nomic" / "tokenizer.json"))
        sess = OnnxSession(str(mdir / "nomic" / "onnx" / "model_int8.onnx"))

        def nomic(texts, is_query):
            prefix = "search_query: " if is_query else "search_document: "
            results = []
            for text in texts:
                ids, mask = tok.encode(prefix + text, MAX_TOKENS)
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                m = mask.astype(np.float32)
                pooled = (hidden * m[:, :, None]).sum(axis=1) / m.sum(axis=1, keepdims=True)
                results.append(l2(pooled[0]).astype(np.float32))
            return np.stack(results)
        return nomic, "dense"

    if name in ("bekko", "bekko-a8m"):
        model_dir = mdir / name
        return mean_pool_dense(
            str(model_dir / "onnx" / "model.onnx"),
            str(model_dir / "tokenizer.json"),
        ), "dense"

    if name == "mlateon":
        tok = TokenizerWrapper(str(mdir / "mlateon" / "tokenizer.json"))
        sess = OnnxSession(str(mdir / "mlateon" / "model_int8.onnx"))
        PREFIX = {True: 256000, False: 256001}

        def mlateon(texts, is_query):
            results = []
            for text in texts:
                ids, mask = tok.encode(text, MAX_TOKENS, PREFIX[is_query])
                hidden = np.asarray(sess.run(ids, mask), dtype=np.float32)
                results.append(hidden[0])
            return results
        return mlateon, "late"

    raise ValueError(f"unknown model {name}")


def bm25_vectors(texts):
    """Reference BM25 vectors (k1=1.2, b=0.75) over a unicode word tokenizer."""
    import re

    tokenized = [re.findall(r"\w+", text.lower()) for text in texts]
    df = {}
    for tokens in tokenized:
        for token in set(tokens):
            df[token] = df.get(token, 0) + 1
    n = len(texts)
    avgdl = sum(len(t) for t in tokenized) / max(n, 1)
    vectors = []
    for tokens in tokenized:
        dl = len(tokens)
        tf = {}
        for token in tokens:
            tf[token] = tf.get(token, 0) + 1
        weights = {}
        for token, count in tf.items():
            idf = math.log(1 + (n - df.get(token, 0) + 0.5) / (df.get(token, 0) + 0.5))
            weights[token] = idf * (count * 1.2 + 1) / (
                count + 1.2 * (1 - 0.75 + 0.75 * dl / max(avgdl, 1))
            )
        if weights:
            terms = np.asarray([hash(t) % (2**31 - 1) for t in weights], dtype=np.int64)
            vals = np.asarray(list(weights.values()), dtype=np.float64)
            vectors.append((terms, vals))
        else:
            vectors.append((np.zeros(0, dtype=np.int64), np.zeros(0, dtype=np.float64)))
    return vectors


def sparse_dot(query, corpus):
    import scipy.sparse as sp

    vocab = {}
    rows, cols, data = [], [], []
    for qidx, (terms, weights) in enumerate(query):
        for term, weight in zip(terms.tolist(), weights.tolist()):
            if term not in vocab:
                vocab[term] = len(vocab)
            rows.append(qidx)
            cols.append(vocab[term])
            data.append(weight)
    q_mat = sp.csr_matrix((data, (rows, cols)), shape=(len(query), len(vocab)))
    rows, cols, data = [], [], []
    for didx, (terms, weights) in enumerate(corpus):
        for term, weight in zip(terms.tolist(), weights.tolist()):
            if term in vocab:
                rows.append(didx)
                cols.append(vocab[term])
                data.append(weight)
    d_mat = sp.csr_matrix((data, (rows, cols)), shape=(len(corpus), len(vocab)))
    return (q_mat @ d_mat.T).toarray()


def maxsim_scores(query_tokens, corpus_tokens):
    """ColBERT MaxSim: sum over query tokens of the max dot over doc tokens.

    Vectorized over the corpus in length-bucketed chunks so the einsum pads
    only to each chunk's real token length, not the corpus maximum.
    """
    dim = corpus_tokens[0].shape[1]
    scores = np.empty((len(query_tokens), len(corpus_tokens)), dtype=np.float32)
    order = sorted(range(len(corpus_tokens)), key=lambda i: corpus_tokens[i].shape[0])
    chunk_size = 1024
    for start in range(0, len(order), chunk_size):
        indices = order[start : start + chunk_size]
        max_len = max(corpus_tokens[i].shape[0] for i in indices)
        corpus = np.zeros((len(indices), max_len, dim), dtype=np.float32)
        for j, i in enumerate(indices):
            doc = corpus_tokens[i].astype(np.float32)
            corpus[j, : doc.shape[0]] = doc
        for qidx, q in enumerate(query_tokens):
            q = q.astype(np.float32)
            dots = np.einsum("qk,dtk->qtd", q, corpus)  # [q_tok, d_tok, n_docs]
            scores[qidx, indices] = dots.max(axis=1).sum(axis=0)
    return scores


# ---------------------------------------------------------------------------
# Metrics (BEIR conventions, binary gains)
# ---------------------------------------------------------------------------


def ndcg_at_k(ranked, relevant, k):
    if not relevant:
        return 0.0
    dcg = sum(1.0 / math.log2(i + 2) for i, doc in enumerate(ranked[:k]) if doc in relevant)
    idcg = sum(1.0 / math.log2(i + 2) for i in range(min(len(relevant), k)))
    return dcg / idcg if idcg > 0 else 0.0


def evaluate(scores, query_ids, passage_ids, qrels):
    metrics = {"nDCG@10": [], "MRR@10": [], "Recall@10": [], "Recall@100": []}
    for qidx, qid in enumerate(query_ids):
        relevant = {doc_id for (q, doc_id) in qrels if q == qid}
        if not relevant:
            # Queries without judgments are excluded (standard practice).
            continue
        ranked = [passage_ids[i] for i in np.argsort(-scores[qidx])]
        metrics["nDCG@10"].append(ndcg_at_k(ranked, relevant, 10))
        for i, doc in enumerate(ranked[:10]):
            if doc in relevant:
                metrics["MRR@10"].append(1.0 / (i + 1))
                break
        else:
            metrics["MRR@10"].append(0.0)
        metrics["Recall@10"].append(
            len(set(ranked[:10]) & relevant) / len(relevant) if relevant else 0.0
        )
        metrics["Recall@100"].append(
            len(set(ranked[:100]) & relevant) / len(relevant) if relevant else 0.0
        )
    if not metrics["nDCG@10"]:
        return {name: 0.0 for name in metrics}
    return {name: float(np.mean(vals)) for name, vals in metrics.items()}


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def cache_file(cache_path, kind):
    """The on-disk cache path: numpy appends .npz/.npy extensions."""
    return cache_path.with_suffix(".npy" if kind == "late" else ".npz")


def encode_all(encoder, kind, texts, is_query, cache_path):
    cache_path = cache_file(cache_path, kind)
    if cache_path.exists():
        if kind == "late":
            return list(np.load(cache_path, allow_pickle=True))
        with np.load(cache_path, allow_pickle=True) as data:
            vectors = data["vectors"]
            return list(vectors) if kind == "sparse" else vectors
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    if kind == "late":
        with ThreadPoolExecutor(max_workers=BATCH_WORKERS) as pool:
            vectors = list(pool.map(lambda t: encoder([t], is_query)[0], texts))
        np.save(cache_path, np.asarray(vectors, dtype=object), allow_pickle=True)
        return vectors
    with ThreadPoolExecutor(max_workers=BATCH_WORKERS) as pool:
        chunks = [texts[i : i + CHUNK] for i in range(0, len(texts), CHUNK)]
        results = list(pool.map(lambda c: encoder(c, is_query), chunks))
    if kind == "sparse":
        vectors = [pair for chunk in results for pair in chunk]
        np.savez_compressed(cache_path, vectors=np.asarray(vectors, dtype=object), allow_pickle=True)
        return vectors
    vectors = np.concatenate(results, axis=0)
    np.savez_compressed(cache_path, vectors=vectors)
    return vectors


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--langs", nargs="+", default=["en", "fr"])
    parser.add_argument("--queries", type=int, default=200)
    parser.add_argument("--passages", type=int, default=5000)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--force", action="store_true")
    parser.add_argument(
        "--models",
        nargs="+",
        default=["bm25", "splade", "minilm", "minilm-l12", "e5-small", "bge-m3", "lfm25", "mdenseon", "mlateon", "bekko", "bekko-a8m", "nomic"],
    )
    args = parser.parse_args()

    report = {
        "methodology": {
            "datasets": {"en": "msmarco-passage/dev", "fr": "mmarco/fr/dev"},
            "sampling": {"queries": args.queries, "passages": args.passages, "seed": args.seed},
            "metrics": ["nDCG@10", "MRR@10", "Recall@10", "Recall@100"],
            "encoding": {
                "bm25": "reference BM25 k1=1.2 b=0.75 (tantivy defaults), unicode word tokens",
                "splade": "query:/document: templates, log1p(ReLU), top-256",
                "minilm": "mean pooling, L2, 256 tokens",
                "e5-small": "query:/passage: prefixes, mean pooling, L2, 512 tokens",
                "bge-m3": "CLS pooling, L2, 512 tokens (dense only; sparse head not released)",
                "lfm25": "query:/document: prefixes, CLS pooling, L2, 512 tokens (bidirectional patch; int8)",
                "mlateon": "[Q]/[D] prefix tokens, MaxSim late interaction, 512 tokens",
                "bekko": "no prefix, mean pooling, L2, 512 tokens (Matryoshka 384-d)",
                "bekko-a8m": "no prefix, mean pooling, L2, 512 tokens (Matryoshka 384-d, 8M active)",
                "nomic": "search_query:/search_document: prefixes, mean pooling, L2, 512 tokens",
                "minilm-l12": "mean pooling, L2, 256 tokens",
                "mdenseon": "query:/document: prefixes, CLS pooling, L2, 512 tokens (int8)",
            },
            "note": (
                "sampled corpora and queries for local CPU feasibility; the upstream "
                "standard (BEIR/MTEB) uses full corpora with the same metrics"
            ),
        },
        "results": {},
    }

    for lang in args.langs:
        rng = np.random.default_rng(args.seed)
        print(f"== loading {lang} dataset", flush=True)
        queries, passages, qrels = load_dataset(lang, rng, args.queries, args.passages)
        query_ids = [q.query_id for q in queries]
        passage_ids = [d.doc_id for d in passages]
        query_texts = [q.text for q in queries]
        passage_texts = [d.text for d in passages]
        report["results"][lang] = {
            "n_queries": len(queries),
            "n_passages": len(passages),
            "n_qrels": len(qrels),
        }

        for model in args.models:
            print(f"== {lang}: {model}", flush=True)
            t0 = time.time()
            encoder, kind = make_encoder(model)
            tag = f"v3_{lang}_c{args.passages}_q{args.queries}_s{args.seed}"
            corpus_vectors = encode_all(
                encoder, kind, passage_texts, False, CACHE_DIR / f"{model}_{tag}_corpus"
            )
            query_vectors = encode_all(
                encoder, kind, query_texts, True, CACHE_DIR / f"{model}_{tag}_queries"
            )
            if kind == "sparse":
                scores = sparse_dot(query_vectors, corpus_vectors)
            elif kind == "dense":
                scores = np.asarray(query_vectors, dtype=np.float32) @ np.asarray(
                    corpus_vectors, dtype=np.float32
                ).T
            else:
                scores = maxsim_scores(query_vectors, corpus_vectors)
            metrics = evaluate(scores, query_ids, passage_ids, qrels)
            metrics["encode_seconds"] = round(time.time() - t0, 1)
            report["results"][lang][model] = metrics
            print(f"   {metrics}", flush=True)

    REPORT_DIR.mkdir(parents=True, exist_ok=True)
    out = REPORT_DIR / "model-retrieval.json"
    out.write_text(json.dumps(report, indent=2) + "\n")
    print(f"report: {out}")


if __name__ == "__main__":
    main()
