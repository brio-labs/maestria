#!/usr/bin/env python3
"""Serve a local SigLIP ONNX model through Maestria's visual vector contract."""

from __future__ import annotations

import argparse
import base64
import binascii
import json
import sys
from io import BytesIO
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Protocol, cast

MAX_BODY_BYTES = 16 * 1024 * 1024
VISUAL_PATH = "/v1/embeddings"
VECTOR_DIMENSIONS = 768
IMAGE_SIZE = 224
TEXT_LENGTH = 64


class VisualEngine(Protocol):
    def embed_text(self, text: str) -> list[float]: ...

    def embed_image(self, image_bytes: bytes) -> list[float]: ...


def decode_data_url(value: str) -> bytes:
    prefix, separator, encoded = value.partition(",")
    if separator != "," or ";base64" not in prefix:
        raise ValueError("visual bytes must be a base64 data URL")
    try:
        return base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError("visual bytes data URL is not valid base64") from error


def input_from_request(payload: dict[str, Any]) -> tuple[str, bytes | None]:
    value = payload.get("input")
    if isinstance(value, str):
        if not value.strip():
            raise ValueError("visual text input must not be empty")
        return value, None
    if not isinstance(value, dict) or value.get("kind") != "visual_source":
        raise ValueError("visual input must be text or a visual_source object")
    encoded = value.get("bytes")
    if not isinstance(encoded, str):
        raise ValueError("visual_source must contain bytes")
    return "", decode_data_url(encoded)


def vector_response(model: str, vector: list[float]) -> bytes:
    if len(vector) != VECTOR_DIMENSIONS:
        raise ValueError(f"visual vector must contain {VECTOR_DIMENSIONS} values")
    if any(not isinstance(value, (int, float)) or not (-float("inf") < value < float("inf")) for value in vector):
        raise ValueError("visual vector must contain finite values")
    return json.dumps(
        {"object": "list", "model": model, "data": [{"index": 0, "embedding": vector}]},
        separators=(",", ":"),
    ).encode("utf-8")


def run_embedding(engine: VisualEngine, payload: dict[str, Any]) -> list[float]:
    text, image_bytes = input_from_request(payload)
    if image_bytes is None:
        return engine.embed_text(text)
    return engine.embed_image(image_bytes)


class SiglipOnnxEngine:
    def __init__(self, vision_model: str, text_model: str, tokenizer_path: str) -> None:
        import numpy as np
        import onnxruntime as ort
        from tokenizers import Tokenizer

        self._np = np
        self._vision = ort.InferenceSession(vision_model, providers=["CPUExecutionProvider"])
        self._text = ort.InferenceSession(text_model, providers=["CPUExecutionProvider"])
        self._tokenizer = Tokenizer.from_file(tokenizer_path)
        vocab = self._tokenizer.get_vocab()
        self._pad_id = vocab.get("<pad>", 0)

    def embed_text(self, text: str) -> list[float]:
        encoding = self._tokenizer.encode(text)
        ids = encoding.ids[:TEXT_LENGTH]
        mask = [1] * len(ids)
        ids.extend([self._pad_id] * (TEXT_LENGTH - len(ids)))
        mask.extend([0] * (TEXT_LENGTH - len(mask)))
        inputs = {
            "input_ids": self._np.asarray([ids], dtype=self._np.int64),
            "attention_mask": self._np.asarray([mask], dtype=self._np.int64),
        }
        return self._normalise(self._text.run(None, inputs)[0][0])

    def embed_image(self, image_bytes: bytes) -> list[float]:
        from PIL import Image

        with Image.open(BytesIO(image_bytes)) as image:
            rgb = image.convert("RGB").resize((IMAGE_SIZE, IMAGE_SIZE), Image.Resampling.BICUBIC)
        pixels = self._np.asarray(rgb, dtype=self._np.float32) / 255.0
        pixels = (pixels - 0.5) / 0.5
        pixels = self._np.transpose(pixels, (2, 0, 1))[None, ...]
        return self._normalise(self._vision.run(None, {"pixel_values": pixels})[0][0])

    def _normalise(self, vector: Any) -> list[float]:
        values = self._np.asarray(vector, dtype=self._np.float32).reshape(-1)
        norm = self._np.linalg.norm(values)
        if not self._np.isfinite(norm) or norm == 0:
            raise ValueError("visual model returned a zero or non-finite vector")
        return (values / norm).tolist()


class VisualServer(ThreadingHTTPServer):
    visual_engine: VisualEngine
    visual_model: str


class RequestHandler(BaseHTTPRequestHandler):
    server_version = "maestria-siglip-visual/1"

    def do_POST(self) -> None:
        if self.path != VISUAL_PATH:
            self.send_error(HTTPStatus.NOT_FOUND, "unknown visual endpoint")
            return
        try:
            content_length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            self.send_error(HTTPStatus.BAD_REQUEST, "invalid Content-Length")
            return
        if content_length < 0 or content_length > MAX_BODY_BYTES:
            self.send_error(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "request body is too large")
            return
        server = cast(VisualServer, self.server)
        try:
            payload = json.loads(self.rfile.read(content_length))
            vector = run_embedding(server.visual_engine, payload)
            body = vector_response(server.visual_model, vector)
        except (ValueError, json.JSONDecodeError) as error:
            self.send_error(HTTPStatus.BAD_REQUEST, str(error))
            return
        except Exception as error:
            self.send_error(HTTPStatus.BAD_GATEWAY, f"visual model failed: {error}")
            return
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def build_server(host: str, port: int, model: str, vision_model: str, text_model: str, tokenizer: str) -> VisualServer:
    server = VisualServer((host, port), RequestHandler)
    server.visual_engine = SiglipOnnxEngine(vision_model, text_model, tokenizer)
    server.visual_model = model
    return server


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", default=10001, type=int)
    parser.add_argument("--model", default="siglip-base-patch16-224-int8")
    parser.add_argument("--vision-model", required=True)
    parser.add_argument("--text-model", required=True)
    parser.add_argument("--tokenizer", required=True)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    server = build_server(args.host, args.port, args.model, args.vision_model, args.text_model, args.tokenizer)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        return 0
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
