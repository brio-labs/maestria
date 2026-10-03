#!/usr/bin/env python3
"""Serve RapidOCR through Maestria's loopback OCR transport contract."""

from __future__ import annotations

import argparse
import base64
import binascii
import json
import sys
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Protocol, cast

MAX_BODY_BYTES = 16 * 1024 * 1024
OCR_PATH = "/v1/chat/completions"


class OcrEngine(Protocol):
    def __call__(self, image: Any) -> tuple[Any, Any]: ...


def decode_data_url(value: str) -> bytes:
    prefix, separator, encoded = value.partition(",")
    if separator != "," or not prefix.startswith("data:image/") or ";base64" not in prefix:
        raise ValueError("image_url must contain a base64 image data URL")
    try:
        return base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ValueError("image data URL is not valid base64") from error


def image_data_from_request(payload: dict[str, Any]) -> bytes:
    try:
        messages = payload["messages"]
        content = messages[-1]["content"]
    except (KeyError, IndexError, TypeError) as error:
        raise ValueError("request must contain a message content list") from error
    if not isinstance(content, list):
        raise ValueError("message content must be a list")
    for item in content:
        if isinstance(item, dict) and item.get("type") == "image_url":
            image_url = item.get("image_url")
            if isinstance(image_url, dict) and isinstance(image_url.get("url"), str):
                return decode_data_url(image_url["url"])
    raise ValueError("request does not contain an image_url data URL")


def ocr_text(result: Any) -> str:
    if not result:
        return ""
    lines: list[str] = []
    for item in result:
        if isinstance(item, (list, tuple)) and len(item) >= 2 and isinstance(item[1], str):
            text = item[1].strip()
            if text:
                lines.append(text)
    return "\n".join(lines)


def run_ocr(engine: OcrEngine, image_bytes: bytes) -> str:
    import cv2
    import numpy as np

    image = cv2.imdecode(np.frombuffer(image_bytes, dtype=np.uint8), cv2.IMREAD_COLOR)
    if image is None:
        raise ValueError("image data could not be decoded")
    result, _ = engine(image)
    return ocr_text(result)


def completion_response(model: str, text: str) -> bytes:
    response = {
        "id": "rapidocr-local",
        "object": "chat.completion",
        "model": model,
        "choices": [
            {
                "index": 0,
                "message": {"role": "assistant", "content": text},
                "finish_reason": "stop",
            }
        ],
    }
    return json.dumps(response, separators=(",", ":")).encode("utf-8")


class RapidOcrServer(ThreadingHTTPServer):
    ocr_engine: OcrEngine
    ocr_model: str


class RequestHandler(BaseHTTPRequestHandler):
    server_version = "maestria-rapidocr/1"

    def do_POST(self) -> None:
        if self.path != OCR_PATH:
            self.send_error(HTTPStatus.NOT_FOUND, "unknown OCR endpoint")
            return
        try:
            content_length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            self.send_error(HTTPStatus.BAD_REQUEST, "invalid Content-Length")
            return
        if content_length < 0 or content_length > MAX_BODY_BYTES:
            self.send_error(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "request body is too large")
            return
        server = cast(RapidOcrServer, self.server)
        try:
            payload = json.loads(self.rfile.read(content_length))
            image_bytes = image_data_from_request(payload)
            text = run_ocr(server.ocr_engine, image_bytes)
            body = completion_response(server.ocr_model, text)
        except (ValueError, json.JSONDecodeError) as error:
            self.send_error(HTTPStatus.BAD_REQUEST, str(error))
            return
        except Exception as error:
            self.send_error(HTTPStatus.BAD_GATEWAY, f"RapidOCR failed: {error}")
            return
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: object) -> None:
        return


def build_server(host: str, port: int, model: str) -> RapidOcrServer:
    from rapidocr_onnxruntime import RapidOCR

    server = RapidOcrServer((host, port), RequestHandler)
    server.ocr_engine = RapidOCR()
    server.ocr_model = model
    return server


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", default=10000, type=int)
    parser.add_argument("--model", default="rapidocr-onnxruntime-1.4.4")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    server = build_server(args.host, args.port, args.model)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        return 0
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
