from __future__ import annotations

import json
import unittest

try:
    from .splade_server import (
        DOCUMENT_TEMPLATE,
        MAX_TEXT_LENGTH,
        QUERY_TEMPLATE,
        input_from_request,
        run_sparse,
        sparse_response,
    )
except ImportError:
    from splade_server import (
        DOCUMENT_TEMPLATE,
        MAX_TEXT_LENGTH,
        QUERY_TEMPLATE,
        input_from_request,
        run_sparse,
        sparse_response,
    )


class RecordingEngine:
    def __init__(self) -> None:
        self.seen: list[tuple[str, str]] = []

    def encode(self, text: str, kind: str) -> tuple[list[int], list[float]]:
        self.seen.append((text, kind))
        return [1, 5, 9], [0.5, 0.25, 0.125]


class SpladeProtocolTests(unittest.TestCase):
    def test_rejects_missing_text(self) -> None:
        with self.assertRaises(ValueError):
            input_from_request({"kind": "query"})

    def test_rejects_empty_text(self) -> None:
        with self.assertRaises(ValueError):
            input_from_request({"text": "   ", "kind": "query"})

    def test_rejects_oversized_text(self) -> None:
        with self.assertRaises(ValueError):
            input_from_request({"text": "a" * (MAX_TEXT_LENGTH + 1), "kind": "query"})

    def test_rejects_unknown_kind(self) -> None:
        with self.assertRaises(ValueError):
            input_from_request({"text": "explain borrow checking", "kind": "vector"})

    def test_accepts_query_and_document_kinds(self) -> None:
        self.assertEqual(
            input_from_request({"text": "explain borrow checking", "kind": "query"}),
            ("explain borrow checking", "query"),
        )
        self.assertEqual(
            input_from_request({"text": "fn main() {}", "kind": "document"}),
            ("fn main() {}", "document"),
        )

    def test_query_template_applied_server_side(self) -> None:
        engine = RecordingEngine()
        run_sparse(engine, {"text": "borrow checking", "kind": "query"}, 256)
        self.assertEqual(engine.seen, [("query: borrow checking", "query")])
        self.assertEqual(QUERY_TEMPLATE, "query: {text}")

    def test_document_template_applied_server_side(self) -> None:
        engine = RecordingEngine()
        run_sparse(engine, {"text": "fn main", "kind": "document"}, 256)
        self.assertEqual(engine.seen, [("document: fn main", "document")])
        self.assertEqual(DOCUMENT_TEMPLATE, "document: {text}")

    def test_response_round_trips_term_vector(self) -> None:
        body = sparse_response("splade", [1, 5, 9], [0.5, 0.25, 0.125], 256)
        payload = json.loads(body)
        self.assertEqual(payload["model"], "splade")
        self.assertEqual(payload["term_ids"], [1, 5, 9])
        self.assertEqual(payload["weights"], [0.5, 0.25, 0.125])

    def test_response_rejects_unsorted_term_ids(self) -> None:
        with self.assertRaises(ValueError):
            sparse_response("splade", [5, 1], [0.5, 0.25], 256)

    def test_response_rejects_duplicate_term_ids(self) -> None:
        with self.assertRaises(ValueError):
            sparse_response("splade", [1, 1], [0.5, 0.25], 256)

    def test_response_rejects_non_positive_weight(self) -> None:
        with self.assertRaises(ValueError):
            sparse_response("splade", [1], [0.0], 256)

    def test_response_rejects_term_cap_excess(self) -> None:
        with self.assertRaises(ValueError):
            sparse_response("splade", list(range(5)), [1.0] * 5, 4)

    def test_response_rejects_mismatched_lengths(self) -> None:
        with self.assertRaises(ValueError):
            sparse_response("splade", [1], [0.5, 0.25], 256)


if __name__ == "__main__":
    unittest.main()
