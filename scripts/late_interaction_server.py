#!/usr/bin/env python3
"""Serve the frozen mLateOn export through the bounded late-interaction wire API.

The process is intentionally local-only, CPU-only, and dependency-lazy. Missing
ONNX dependencies fail startup instead of silently switching models or making
network requests.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import queue
import socket
import sys
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Any

MAX_REQUEST_BYTES = 128 * 1024
MAX_RESPONSE_BYTES = 2 * 1024 * 1024
MAX_QUERY_BYTES = 8192
MAX_DOCUMENT_BYTES = 65536
MAX_DIMENSIONS = 4096
HEALTH_PATH = "/health"
MULTIVECTOR_PATH = "/v1/multivector"


class LateInteractionError(Exception):
    """A safe, non-content-bearing request or inference error."""


class LateOnxEngine:
    def __init__(self, model_dir: Path, profile: dict[str, Any], threads: int) -> None:
        try:
            import numpy as np
            import onnxruntime as ort
            from tokenizers import Tokenizer
        except ImportError as error:
            raise LateInteractionError(f"required local runtime dependency unavailable: {error.name}") from error
        self._np = np
        self.profile = profile
        tokenizer_path = model_dir / "tokenizer.json"
        model_path = model_dir / "model_int8.onnx"
        self._tokenizer = Tokenizer.from_file(str(tokenizer_path))
        options = ort.SessionOptions()
        options.intra_op_num_threads = threads
        self._session = ort.InferenceSession(
            str(model_path), sess_options=options, providers=["CPUExecutionProvider"]
        )
        self._input_names = {item.name for item in self._session.get_inputs()}
        output_dim = int(profile["output_dimensions"])
        if output_dim <= 0 or output_dim > MAX_DIMENSIONS:
            raise LateInteractionError("profile output dimension is outside the safety ceiling")
        self._dim = output_dim

    def encode(self, text: str, kind: str, max_vectors: int, deadline: float) -> tuple[int, bool, list[dict[str, Any]]]:
        if time.monotonic() >= deadline:
            raise LateInteractionError("request deadline exceeded")
        if kind not in ("query", "document"):
            raise LateInteractionError("unknown input kind")
        limit = int(self.profile["query_max_vectors"] if kind == "query" else self.profile["document_max_vectors"])
        if max_vectors != limit:
            raise LateInteractionError("requested vector cap does not match profile")
        prefix = self.profile["query_prefix"] if kind == "query" else self.profile["document_prefix"]
        encoded = self._tokenizer.encode(text)
        raw_ids = [int(prefix["id"])] + [int(token_id) for token_id in encoded.ids]
        original_count = len(raw_ids)
        retained_ids = raw_ids[:limit]
        truncated = len(retained_ids) < original_count
        pad_id = int(self.profile["pad_token_id"])
        mask_values = [0 if token_id == pad_id else 1 for token_id in retained_ids]
        ids = self._np.asarray([retained_ids], dtype=self._np.int64)
        mask = self._np.asarray([mask_values], dtype=self._np.int64)
        feed: dict[str, Any] = {"input_ids": ids}
        if "input_mask" in self._input_names:
            feed["input_mask"] = mask
        elif "attention_mask" in self._input_names:
            feed["attention_mask"] = mask
        if "segment_ids" in self._input_names:
            feed["segment_ids"] = self._np.zeros_like(ids)
        elif "token_type_ids" in self._input_names:
            feed["token_type_ids"] = self._np.zeros_like(ids)
        if time.monotonic() >= deadline:
            raise LateInteractionError("request deadline exceeded")
        output = self._np.asarray(self._session.run(None, feed)[0])
        if output.ndim != 3 or output.shape[0] != 1 or output.shape[1] != len(retained_ids) or output.shape[2] != self._dim:
            raise LateInteractionError("model output shape does not match profile")
        vectors: list[dict[str, Any]] = []
        for position, row in enumerate(output[0]):
            if mask_values[position] == 0:
                continue
            values = self._np.asarray(row, dtype=self._np.float64)
            if not self._np.isfinite(values).all():
                raise LateInteractionError("model returned a non-finite vector")
            norm = float(self._np.sqrt(self._np.dot(values, values)))
            if not math.isfinite(norm) or norm <= 0.0:
                raise LateInteractionError("model returned a zero or invalid vector")
            normalized = (values / norm).astype(self._np.float32)
            vectors.append({"position": position, "embedding": [float(value) for value in normalized]})
        if not vectors:
            raise LateInteractionError("model returned no attended vectors")
        if time.monotonic() >= deadline:
            raise LateInteractionError("request deadline exceeded")
        return original_count, truncated, vectors


class BoundedWorkQueue:
    def __init__(self, workers: int) -> None:
        self._queue: queue.Queue[tuple[socket.socket, Any] | None] = queue.Queue(maxsize=1)
        self._threads = [threading.Thread(target=self._run, daemon=True) for _ in range(workers)]
        for worker in self._threads:
            worker.start()

    def submit(self, request: socket.socket, client_address: Any) -> bool:
        try:
            self._queue.put_nowait((request, client_address))
            return True
        except queue.Full:
            return False

    def _run(self) -> None:
        while True:
            item = self._queue.get()
            if item is None:
                return
            request, client_address = item
            try:
                self._server.finish_request(request, client_address)
            except Exception:
                self._server.handle_error(request, client_address)
            finally:
                self._server.shutdown_request(request)
                self._queue.task_done()

    def bind(self, server: "LateServer") -> None:
        self._server = server


class LateServer(HTTPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, address: tuple[str, int], profile: dict[str, Any], engine: LateOnxEngine, workers: int) -> None:
        super().__init__(address, LateRequestHandler)
        self.profile = profile
        self.engine = engine
        self.profile_digest = profile["artifact_hash"]
        self.work = BoundedWorkQueue(workers)
        self.work.bind(self)

    def process_request(self, request: socket.socket, client_address: Any) -> None:
        if not self.work.submit(request, client_address):
            try:
                request.sendall(
                    b"HTTP/1.1 503 Service Unavailable\r\n"
                    b"Content-Type: application/json\r\n"
                    b"Content-Length: 45\r\n"
                    b"Connection: close\r\n\r\n"
                    b'{"error":"bounded worker queue is full"}'
                )
            finally:
                request.close()

    def server_close(self) -> None:
        super().server_close()


class LateRequestHandler(BaseHTTPRequestHandler):
    server: LateServer
    server_version = "maestria-late-interaction/1"

    def log_message(self, _format: str, *_args: Any) -> None:
        # Request/response content and timing are deliberately not logged.
        return

    def _send_json(self, status: HTTPStatus, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        if len(body) > MAX_RESPONSE_BYTES:
            body = b'{"error":"response exceeds safety limit"}'
            status = HTTPStatus.INTERNAL_SERVER_ERROR
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        if self.path != HEALTH_PATH:
            self._send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        self._send_json(HTTPStatus.OK, {"ready": True, "profile_digest": self.server.profile_digest})

    def do_POST(self) -> None:
        if self.path != MULTIVECTOR_PATH:
            self._send_json(HTTPStatus.NOT_FOUND, {"error": "not found"})
            return
        content_length = self.headers.get("Content-Length")
        try:
            declared = int(content_length) if content_length is not None else -1
        except ValueError:
            declared = -1
        if declared < 0 or declared > MAX_REQUEST_BYTES:
            self._send_json(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, {"error": "request body exceeds safety limit"})
            return
        body = self.rfile.read(declared)
        if len(body) != declared:
            self._send_json(HTTPStatus.BAD_REQUEST, {"error": "request body is truncated"})
            return
        try:
            payload = json.loads(body.decode("utf-8"), object_pairs_hook=self._reject_duplicate_keys)
            if not isinstance(payload, dict):
                raise LateInteractionError("request must be an object")
            self._handle_payload(payload)
        except (UnicodeDecodeError, json.JSONDecodeError, LateInteractionError, OSError) as error:
            self._send_json(HTTPStatus.BAD_REQUEST, {"error": str(error)})

    @staticmethod
    def _reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise LateInteractionError("duplicate request field")
            result[key] = value
        return result

    def _handle_payload(self, payload: dict[str, Any]) -> None:
        allowed = {
            "schema_version", "model", "fingerprint", "profile_identity", "identity",
            "kind", "input_hash", "text", "max_vectors", "remaining_ms", "source",
        }
        unknown = set(payload) - allowed
        if unknown:
            raise LateInteractionError("unknown request field")
        if payload.get("schema_version") != 1:
            raise LateInteractionError("unsupported wire schema")
        profile = self.server.profile
        if payload.get("model") != profile.get("model"):
            raise LateInteractionError("model does not match loaded profile")
        if payload.get("fingerprint") != profile.get("artifact_hash"):
            raise LateInteractionError("fingerprint does not match loaded profile")
        if payload.get("profile_identity") != profile.get("artifact_hash"):
            raise LateInteractionError("profile identity does not match loaded profile")
        identity = payload.get("identity")
        if (
            not isinstance(identity, str)
            or len(identity) != len("sha256:" + "0" * 64)
            or not identity.startswith("sha256:")
        ):
            raise LateInteractionError("complete identity digest is invalid")
        kind = payload.get("kind")
        text = payload.get("text")
        input_hash = payload.get("input_hash")
        max_vectors = payload.get("max_vectors")
        remaining_ms = payload.get("remaining_ms")
        if kind not in ("query", "document") or not isinstance(text, str) or not text.strip():
            raise LateInteractionError("input kind or text is invalid")
        if not isinstance(input_hash, str) or not input_hash.startswith("sha256:"):
            raise LateInteractionError("input hash is invalid")
        if not isinstance(max_vectors, int) or not isinstance(remaining_ms, int) or remaining_ms <= 0:
            raise LateInteractionError("request budget is invalid")
        encoded_size = len(text.encode("utf-8"))
        if encoded_size > (MAX_QUERY_BYTES if kind == "query" else MAX_DOCUMENT_BYTES):
            raise LateInteractionError("input exceeds safety limit")
        if kind == "document":
            source = payload.get("source")
            if not isinstance(source, dict) or not source:
                raise LateInteractionError("document source binding is required")
        deadline = time.monotonic() + min(remaining_ms, 250) / 1000.0
        original_count, truncated, vectors = self.server.engine.encode(text, kind, max_vectors, deadline)
        response: dict[str, Any] = {
            "schema_version": 1,
            "model": profile["model"],
            "fingerprint": profile["artifact_hash"],
            "profile_identity": profile["artifact_hash"],
            "identity": identity,
            "input_hash": input_hash,
            "original_token_count": original_count,
            "truncated": truncated,
            "vectors": vectors,
        }
        if kind == "document":
            response["source"] = payload["source"]
        self._send_json(HTTPStatus.OK, response)


def load_profile(path: Path, profile_name: str | None) -> dict[str, Any]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(document, dict):
        raise LateInteractionError("profile must be an object")
    if isinstance(document.get("profiles"), dict):
        if not profile_name:
            raise LateInteractionError("profile name is required for a profile collection")
        profile = document["profiles"].get(profile_name)
    else:
        profile = document
    if not isinstance(profile, dict):
        raise LateInteractionError("unknown profile")
    required = ("model", "artifact_hash", "output_dimensions", "query_max_vectors", "document_max_vectors")
    if any(key not in profile for key in required):
        raise LateInteractionError("profile is incomplete")
    if not isinstance(profile["artifact_hash"], str) or not profile["artifact_hash"].startswith("sha256:"):
        raise LateInteractionError("profile artifact hash is invalid")
    return profile


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model-dir", required=True, type=Path)
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--profile-name")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8093)
    parser.add_argument("--threads", type=int, default=2)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.host not in ("127.0.0.1", "::1"):
            raise LateInteractionError("sidecar must bind to loopback")
        if args.threads < 1 or args.threads > 2:
            raise LateInteractionError("threads must be between one and two")
        profile = load_profile(args.profile, args.profile_name)
        engine = LateOnxEngine(args.model_dir, profile, args.threads)
        server = LateServer((args.host, args.port), profile, engine, workers=1)
    except (OSError, LateInteractionError, json.JSONDecodeError) as error:
        print(f"late interaction server failed: {error}", file=sys.stderr)
        return 2
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
