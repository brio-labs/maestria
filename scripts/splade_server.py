#!/usr/bin/env python3
"""Serve a local SPLADE-family ONNX model through Maestria's sparse vector contract."""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import sys
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Protocol, cast

MAX_BODY_BYTES = 1024 * 1024
SPARSE_PATH = "/v1/sparse"
SPARSE_BATCH_PATH = "/v1/sparse/batch"
MAX_TEXT_LENGTH = 8192
MAX_BATCH_SIZE = 1024
BATCH_WORKERS = 6
DEFAULT_TERM_CAP = 256
QUERY_TEMPLATE = "query: {text}"
DOCUMENT_TEMPLATE = "document: {text}"
KINDS = ("query", "document")


class SparseEngine(Protocol):
    def encode(self, text: str, kind: str) -> tuple[list[int], list[float]]: ...


def input_from_request(payload: dict[str, Any]) -> tuple[str, str]:
    text = payload.get("text")
    kind = payload.get("kind")
    if not isinstance(text, str) or not text.strip():
        raise ValueError("sparse text input must be a non-empty string")
    if len(text) > MAX_TEXT_LENGTH:
        raise ValueError(f"sparse text input exceeds {MAX_TEXT_LENGTH} characters")
    if kind not in KINDS:
        raise ValueError("sparse kind must be \"query\" or \"document\"")
    return text, kind


def sparse_batch_response(
    model: str, vectors: list[tuple[list[int], list[float]]], term_cap: int
) -> bytes:
    return json.dumps(
        {
            "model": model,
            "term_cap": term_cap,
            "vectors": [
                {"term_ids": term_ids, "weights": weights}
                for term_ids, weights in vectors
            ],
        }
    ).encode("utf-8")


def sparse_response(model: str, term_ids: list[int], weights: list[float], term_cap: int) -> bytes:
    if len(term_ids) != len(weights):
        raise ValueError("sparse term_ids and weights must have equal length")
    if not term_ids:
        raise ValueError("sparse response must contain at least one term")
    if len(term_ids) > term_cap:
        raise ValueError(f"sparse response exceeds term cap {term_cap}")
    previous = -1
    for term_id, weight in zip(term_ids, weights):
        if not isinstance(term_id, int) or term_id < 0 or term_id <= previous:
            raise ValueError("sparse term_ids must be strictly ascending non-negative integers")
        if not isinstance(weight, (int, float)) or not (0.0 < weight < float("inf")):
            raise ValueError("sparse weights must be finite positive values")
        previous = term_id
    return json.dumps(
        {"model": model, "term_ids": term_ids, "weights": weights},
        separators=(",", ":"),
    ).encode("utf-8")


def run_sparse_batch(
    engine: SparseEngine, payload: dict[str, Any], term_cap: int
) -> list[tuple[list[int], list[float]]]:
    if not isinstance(payload.get("texts"), list) or not payload["texts"]:
        raise ValueError("texts must be a non-empty list")
    if len(payload["texts"]) > MAX_BATCH_SIZE:
        raise ValueError("texts exceeds the batch size limit")
    kind = payload.get("kind", "document")
    if kind not in ("query", "document"):
        raise ValueError("kind must be 'query' or 'document'")
    template = QUERY_TEMPLATE if kind == "query" else DOCUMENT_TEMPLATE
    texts = payload["texts"]
    for text in texts:
        if not isinstance(text, str):
            raise ValueError("texts entries must be strings")
        if len(text) > MAX_TEXT_LENGTH:
            raise ValueError("text exceeds the length limit")
    # The ONNX runtime releases the GIL during session.run, so a bounded
    # worker pool parallelizes the transformer passes on the machine's cores
    # while preserving input order in the response.
    def encode_one(text: str) -> tuple[list[int], list[float]]:
        term_ids, weights = engine.encode(template.format(text=text), kind)
        if not term_ids or len(term_ids) != len(weights) or len(term_ids) > term_cap:
            raise ValueError("sparse engine returned an invalid term vector")
        return term_ids, weights

    with concurrent.futures.ThreadPoolExecutor(max_workers=BATCH_WORKERS) as pool:
        return list(pool.map(encode_one, texts))


def run_sparse(engine: SparseEngine, payload: dict[str, Any], term_cap: int) -> tuple[list[int], list[float]]:
    text, kind = input_from_request(payload)
    template = QUERY_TEMPLATE if kind == "query" else DOCUMENT_TEMPLATE
    term_ids, weights = engine.encode(template.format(text=text), kind)
    if not term_ids or len(term_ids) != len(weights) or len(term_ids) > term_cap:
        raise ValueError("sparse engine returned an invalid term vector")
    return term_ids, weights


class SpladeOnnxEngine:
    """SPLADE-family encoder: logits over the token vocabulary become term weights.

    The checkpoint must export the full SPLADE graph (backbone + pooling) so the
    ONNX output is a [batch, vocabulary] logit tensor. Term weights follow the
    standard SPLADE weighting `log(1 + ReLU(logits))`, top-`term_cap` kept, terms
    returned ascending and deduplicated.
    """

    def __init__(self, model_path: str, tokenizer_path: str, term_cap: int = DEFAULT_TERM_CAP) -> None:
        import numpy as np
        import onnxruntime as ort
        from tokenizers import Tokenizer

        self._np = np
        # Bounded intra-op threads leave cores for the batch worker pool;
        # small [1, 512] passes do not scale past two threads anyway.
        options = ort.SessionOptions()
        options.intra_op_num_threads = 2
        self._session = ort.InferenceSession(
            model_path, sess_options=options, providers=["CPUExecutionProvider"]
        )
        self._input_names = {input_.name for input_ in self._session.get_inputs()}
        self._tokenizer = Tokenizer.from_file(tokenizer_path)
        self._term_cap = term_cap
        self._max_tokens = 512

    def encode(self, text: str, kind: str) -> tuple[list[int], list[float]]:
        encoding = self._tokenizer.encode(text)
        ids = encoding.ids
        mask = encoding.attention_mask
        inputs = {}
        if "input_ids" in self._input_names:
            inputs["input_ids"] = self._np.asarray([ids], dtype=self._np.int64)
        mask_name = "input_mask" if "input_mask" in self._input_names else "attention_mask"
        if mask_name in self._input_names:
            inputs[mask_name] = self._np.asarray([mask], dtype=self._np.int64)
        if "segment_ids" in self._input_names:
            inputs["segment_ids"] = self._np.zeros((1, len(ids)), dtype=self._np.int64)
        if "token_type_ids" in self._input_names:
            inputs["token_type_ids"] = self._np.zeros((1, len(ids)), dtype=self._np.int64)
        if not inputs:
            raise ValueError("sparse ONNX session exposes no recognized token inputs")
        logits = self._session.run(None, inputs)[0]
        scores = self._np.asarray(logits[0], dtype=self._np.float32)
        if scores.ndim == 2:
            scores = scores.max(axis=0)
        scores = scores.reshape(-1)
        weights = self._np.log1p(self._np.maximum(scores, 0.0))
        order = self._np.argsort(-weights, kind="stable")
        selected: list[tuple[int, float]] = []
        for index in order.tolist():
            weight = float(weights[index])
            if weight <= 0.0:
                break
            selected.append((int(index), weight))
            if len(selected) >= self._term_cap:
                break
        if not selected:
            raise ValueError("sparse model produced no positive term weights")
        selected.sort()
        return [term_id for term_id, _ in selected], [weight for _, weight in selected]


class SparseServer(ThreadingHTTPServer):
    sparse_engine: SparseEngine
    sparse_model: str
    term_cap: int


class RequestHandler(BaseHTTPRequestHandler):
    server_version = "maestria-splade-sparse/1"

    def do_POST(self) -> None:
        if self.path not in (SPARSE_PATH, SPARSE_BATCH_PATH):
            self.send_error(HTTPStatus.NOT_FOUND, "unknown sparse endpoint")
            return
        try:
            content_length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            self.send_error(HTTPStatus.BAD_REQUEST, "invalid Content-Length")
            return
        if content_length < 0 or content_length > MAX_BODY_BYTES:
            self.send_error(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "request body is too large")
            return
        server = cast(SparseServer, self.server)
        try:
            payload = json.loads(self.rfile.read(content_length))
            if self.path == SPARSE_BATCH_PATH:
                vectors = run_sparse_batch(server.sparse_engine, payload, server.term_cap)
                body = sparse_batch_response(server.sparse_model, vectors, server.term_cap)
            else:
                term_ids, weights = run_sparse(server.sparse_engine, payload, server.term_cap)
                body = sparse_response(server.sparse_model, term_ids, weights, server.term_cap)
        except (ValueError, json.JSONDecodeError) as error:
            self.send_error(HTTPStatus.BAD_REQUEST, str(error))
            return
        except Exception as error:
            self.send_error(HTTPStatus.BAD_GATEWAY, f"sparse model failed: {error}")
            return
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def build_server(
    host: str, port: int, model: str, model_path: str, tokenizer: str, term_cap: int
) -> SparseServer:
    server = SparseServer((host, port), RequestHandler)
    server.sparse_engine = SpladeOnnxEngine(model_path, tokenizer, term_cap)
    server.sparse_model = model
    server.term_cap = term_cap
    return server


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", default=10002, type=int)
    parser.add_argument("--model", default="prithivida/Splade_PP_en_v1")
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--tokenizer", default=None, help="tokenizer.json path (default: MODEL_DIR/tokenizer.json)")
    parser.add_argument("--term-cap", default=DEFAULT_TERM_CAP, type=int)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    model_path = f"{args.model_dir}/onnx/model.onnx"
    tokenizer = args.tokenizer or f"{args.model_dir}/tokenizer.json"
    server = build_server(args.host, args.port, args.model, model_path, tokenizer, args.term_cap)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        return 0
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
