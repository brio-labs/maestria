from __future__ import annotations

import http.client
import json
import threading
import unittest

try:
    from .embedding_server import (
        EMBEDDINGS_PATH,
        MAX_BODY_BYTES,
        build_server,
    )
except ImportError:
    from embedding_server import (
        EMBEDDINGS_PATH,
        MAX_BODY_BYTES,
        build_server,
    )


class FakeEngine:
    def __init__(self) -> None:
        self.seen: list[str] = []

    def embed(self, text: str) -> list[float]:
        self.seen.append(text)
        if text.strip() == "boom":
            raise ValueError("fake failure")
        return [0.6, 0.8, 0.0]

    def embed_batch(self, texts: list[str]) -> list[list[float]]:
        return [self.embed(text) for text in texts]


class EmbeddingServerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.server = build_server(
            "127.0.0.1", 0, "bekko-embedding-v1-a25m", "model.onnx", "tokenizer.json", "mean", 3,
            engine=FakeEngine(),  # type: ignore[arg-type]
        )
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        host, port = self.server.server_address[:2]
        self.connection = http.client.HTTPConnection(host, port, timeout=10)

    def tearDown(self) -> None:
        self.connection.close()
        self.server.shutdown()
        self.server.server_close()

    def post(self, payload: dict) -> tuple[int, dict]:
        body = json.dumps(payload).encode("utf-8")
        self.connection.request("POST", EMBEDDINGS_PATH, body, {"Content-Type": "application/json"})
        response = self.connection.getresponse()
        raw = response.read()
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError:
            parsed = {"text": raw.decode("utf-8", errors="replace")}
        return response.status, parsed

    def test_embeds_a_text_with_normalized_vector(self) -> None:
        status, data = self.post({"input": "the capital of France", "model": "bekko-embedding-v1-a25m"})
        self.assertEqual(status, 200)
        self.assertEqual(data["model"], "bekko-embedding-v1-a25m")
        self.assertEqual(len(data["data"]), 1)
        self.assertEqual(len(data["data"][0]["embedding"]), 3)

    def test_rejects_unknown_model(self) -> None:
        status, data = self.post({"input": "text", "model": "other-model"})
        self.assertEqual(status, 400)

    def test_rejects_empty_input(self) -> None:
        status, data = self.post({"input": "   ", "model": "bekko-embedding-v1-a25m"})
        self.assertEqual(status, 400)

    def test_accepts_batch_of_strings(self) -> None:
        status, data = self.post(
            {"input": ["first", "second"], "model": "bekko-embedding-v1-a25m"}
        )
        self.assertEqual(status, 200)
        self.assertEqual([item["index"] for item in data["data"]], [0, 1])

    def test_rejects_non_string_list_input(self) -> None:
        status, data = self.post({"input": [1, 2], "model": "bekko-embedding-v1-a25m"})
        self.assertEqual(status, 400)

    def test_rejects_empty_list_input(self) -> None:
        status, data = self.post({"input": [], "model": "bekko-embedding-v1-a25m"})
        self.assertEqual(status, 400)

    def test_rejects_unknown_path(self) -> None:
        self.connection.request("POST", "/v1/other", b"{}", {"Content-Type": "application/json"})
        response = self.connection.getresponse()
        self.assertEqual(response.status, 404)

    def test_engine_failure_is_gateway_error(self) -> None:
        status, data = self.post({"input": "boom", "model": "bekko-embedding-v1-a25m"})
        self.assertEqual(status, 502)

    def test_body_limit_enforced(self) -> None:
        self.connection.putrequest("POST", EMBEDDINGS_PATH)
        self.connection.putheader("Content-Type", "application/json")
        self.connection.putheader("Content-Length", str(MAX_BODY_BYTES + 1))
        self.connection.endheaders()
        response = self.connection.getresponse()
        response.read()
        self.assertEqual(response.status, 413)


if __name__ == "__main__":
    unittest.main()
