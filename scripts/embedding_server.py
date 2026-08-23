#!/usr/bin/env python3
"""Serve a local ONNX embedding model through Maestria's dense vector contract.

The daemon's dense lane talks OpenAI-compatible embeddings: POST
/v1/embeddings with {"input": str, "model": str, "dimensions": n?} returns
{"data": [{"embedding": [f32, ...]}], "model": str}. This sidecar serves any
sentence-transformers-style ONNX checkpoint whose output is token embeddings
([1, seq, dim]); pooling follows the checkpoint's sentence-transformers
configuration (mean or CLS), and embeddings are L2-normalized.

The default profile is the pinned bekko-embedding-v1-a25m (MIT, 100+
languages, 384-dim Matryoshka, no query/document prefixes required).
"""

from __future__ import annotations

import argparse
import json
import sys
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, cast

MAX_BODY_BYTES = 4 * 1024 * 1024
EMBEDDINGS_PATH = "/v1/embeddings"
DEFAULT_MODEL = "bekko-embedding-v1-a25m"
DEFAULT_DIMENSIONS = 384


class EmbeddingEngine:
    """ONNX encoder with mean pooling + L2 normalization (default).

    Pooling and dimensions can be overridden by the checkpoint profile.
    """

    def __init__(self, model_path: str, tokenizer_path: str, pooling: str, dimensions: int) -> None:
        import numpy as np
        import onnxruntime as ort
        from tokenizers import Tokenizer

        self._np = np
        options = ort.SessionOptions()
        options.intra_op_num_threads = 2
        self._session = ort.InferenceSession(
            model_path, sess_options=options, providers=["CPUExecutionProvider"]
        )
        self._input_names = {input_.name for input_ in self._session.get_inputs()}
        self._tokenizer = Tokenizer.from_file(tokenizer_path)
        self._pooling = pooling
        self._dimensions = dimensions

    def embed(self, text: str) -> list[float]:
        np = self._np
        encoding = self._tokenizer.encode(text)
        ids = np.asarray([encoding.ids], dtype=np.int64)
        mask = np.asarray([encoding.attention_mask], dtype=np.int64)
        feed = {"input_ids": ids}
        if "attention_mask" in self._input_names:
            feed["attention_mask"] = mask
        elif "input_mask" in self._input_names:
            feed["input_mask"] = mask
        if "token_type_ids" in self._input_names:
            feed["token_type_ids"] = np.zeros_like(ids)
        elif "segment_ids" in self._input_names:
            feed["segment_ids"] = np.zeros_like(ids)
        hidden = np.asarray(self._session.run(None, feed)[0], dtype=np.float32)
        if hidden.ndim == 3:
            if self._pooling == "cls":
                pooled = hidden[:, 0]
            else:
                m = mask.astype(np.float32)
                pooled = (hidden * m[:, :, None]).sum(axis=1) / m.sum(axis=1, keepdims=True)
            vector = pooled[0]
        else:
            vector = hidden[0]
        norm = float(np.linalg.norm(vector))
        if norm <= 0.0 or not np.all(np.isfinite(vector)):
            raise ValueError("embedding model produced an invalid vector")
        vector = vector / norm
        if len(vector) != self._dimensions:
            raise ValueError(
                f"embedding dimension mismatch: expected {self._dimensions}, got {len(vector)}"
            )
        return [float(value) for value in vector]

    def embed_batch(self, texts: list[str]) -> list[list[float]]:
        """Encode a batch in one padded session run; pool and normalize rows.

        Padding to the longest sequence in the batch lets the ONNX session
        process every text in one run instead of one run per chunk.
        """
        np = self._np
        encodings = [self._tokenizer.encode(text) for text in texts]
        max_len = max(len(encoding.ids) for encoding in encodings)
        rows = len(encodings)
        ids = np.zeros((rows, max_len), dtype=np.int64)
        mask = np.zeros((rows, max_len), dtype=np.int64)
        for row, encoding in enumerate(encodings):
            length = len(encoding.ids)
            ids[row, :length] = encoding.ids
            mask[row, :length] = encoding.attention_mask
        feed = {"input_ids": ids}
        if "attention_mask" in self._input_names:
            feed["attention_mask"] = mask
        elif "input_mask" in self._input_names:
            feed["input_mask"] = mask
        if "token_type_ids" in self._input_names:
            feed["token_type_ids"] = np.zeros_like(ids)
        elif "segment_ids" in self._input_names:
            feed["segment_ids"] = np.zeros_like(ids)
        hidden = np.asarray(self._session.run(None, feed)[0], dtype=np.float32)
        if hidden.ndim != 3 or hidden.shape[0] != rows:
            raise ValueError("embedding model produced unexpected output shape")
        vectors: list[list[float]] = []
        for row in range(rows):
            pooled = hidden[row, 0] if self._pooling == "cls" else (
                (hidden[row] * mask[row].astype(np.float32)[:, None]).sum(axis=0)
                / max(float(mask[row].sum()), 1.0)
            )
            norm = float(np.linalg.norm(pooled))
            if norm <= 0.0 or not np.all(np.isfinite(pooled)):
                raise ValueError("embedding model produced an invalid vector")
            normalized = pooled / norm
            if len(normalized) != self._dimensions:
                raise ValueError(
                    f"embedding dimension mismatch: expected {self._dimensions}, got {len(normalized)}"
                )
            vectors.append([float(value) for value in normalized])
        return vectors



class EmbeddingServer(ThreadingHTTPServer):
    engine: EmbeddingEngine
    model: str
    dimensions: int


class RequestHandler(BaseHTTPRequestHandler):
    server_version = "maestria-embedding/1"

    def do_POST(self) -> None:
        if self.path != EMBEDDINGS_PATH:
            self.send_error(HTTPStatus.NOT_FOUND, "unknown embeddings endpoint")
            return
        try:
            content_length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            self.send_error(HTTPStatus.BAD_REQUEST, "invalid Content-Length")
            return
        if content_length < 0 or content_length > MAX_BODY_BYTES:
            self.send_error(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "request body is too large")
            return
        server = cast(EmbeddingServer, self.server)
        try:
            payload = json.loads(self.rfile.read(content_length))
            text = payload.get("input")
            if isinstance(text, str):
                if not text.strip():
                    raise ValueError("input must be a non-empty string")
                texts = [text]
            elif isinstance(text, list) and text and all(
                isinstance(item, str) and item.strip() for item in text
            ):
                texts = text
            else:
                raise ValueError(
                    "input must be a non-empty string or a non-empty list of strings"
                )
            requested_model = payload.get("model")
            if requested_model and requested_model != server.model:
                raise ValueError("requested model does not match the served model")
        except (ValueError, json.JSONDecodeError) as error:
            self.send_error(HTTPStatus.BAD_REQUEST, str(error))
            return
        try:
            embeddings = server.engine.embed_batch(texts)
        except Exception as error:
            self.send_error(HTTPStatus.BAD_GATEWAY, f"embedding model failed: {error}")
            return
        body = json.dumps(
            {
                "data": [
                    {"embedding": embedding, "index": index}
                    for index, embedding in enumerate(embeddings)
                ],
                "model": server.model,
            }
        ).encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def build_server(
    host: str,
    port: int,
    model: str,
    model_path: str,
    tokenizer: str,
    pooling: str,
    dimensions: int,
    engine: EmbeddingEngine | None = None,
) -> EmbeddingServer:
    server = EmbeddingServer((host, port), RequestHandler)
    server.engine = engine if engine is not None else EmbeddingEngine(
        model_path, tokenizer, pooling, dimensions
    )
    server.model = model
    server.dimensions = dimensions
    return server


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", default=10003, type=int)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--tokenizer", default=None, help="tokenizer.json path (default: MODEL_DIR/tokenizer.json)")
    parser.add_argument("--pooling", default="mean", choices=("mean", "cls"))
    parser.add_argument("--dimensions", default=DEFAULT_DIMENSIONS, type=int)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    model_path = f"{args.model_dir}/onnx/model.onnx"
    tokenizer = args.tokenizer or f"{args.model_dir}/tokenizer.json"
    server = build_server(
        args.host, args.port, args.model, model_path, tokenizer, args.pooling, args.dimensions
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
