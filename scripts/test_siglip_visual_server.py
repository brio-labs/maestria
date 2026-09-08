from __future__ import annotations

import base64
import json
import unittest

try:
    from .siglip_visual_server import (
        decode_data_url,
        input_from_request,
        run_embedding,
        vector_response,
    )
except ImportError:
    from siglip_visual_server import (
        decode_data_url,
        input_from_request,
        run_embedding,
        vector_response,
    )
class FixtureEngine:
    def embed_text(self, text: str) -> list[float]:
        return [float(len(text))]

    def embed_image(
        self, image: tuple[bytes, tuple[int, int, int, int] | None]
    ) -> list[float]:
        image_bytes, region = image
        return [float(len(image_bytes)) + (sum(region) if region else 0.0)]


class SiglipVisualProtocolTests(unittest.TestCase):
    def test_decodes_base64_image_data_url(self) -> None:
        encoded = base64.b64encode(b"image").decode("ascii")
        self.assertEqual(decode_data_url(f"data:image/png;base64,{encoded}"), b"image")

    def test_rejects_non_base64_data_url(self) -> None:
        with self.assertRaises(ValueError):
            decode_data_url("data:image/png,image")

    def test_accepts_text_and_source_inputs(self) -> None:
        self.assertEqual(input_from_request({"input": "table latency"}), ("table latency", None))
        encoded = base64.b64encode(b"page").decode("ascii")
        self.assertEqual(
            input_from_request(
                {"input": {"source": {"kind": "page", "blob": "b", "page_start": 1, "page_end": 1}, "bytes": f"data:application/octet-stream;base64,{encoded}"}}
            ),
            ("", (b"page", None)),
        )

    def test_region_source_carries_crop_rectangle(self) -> None:
        encoded = base64.b64encode(b"page").decode("ascii")
        self.assertEqual(
            input_from_request(
                {
                    "input": {
                        "source": {
                            "kind": "region",
                            "blob": "b",
                            "page": 1,
                            "x": 100,
                            "y": 220,
                            "width": 900,
                            "height": 300,
                        },
                        "bytes": f"data:image/png;base64,{encoded}",
                    }
                }
            ),
            ("", (b"page", (100, 220, 900, 300))),
        )

    def test_dispatches_each_input_to_the_engine(self) -> None:
        engine = FixtureEngine()
        self.assertEqual(run_embedding(engine, {"input": "abc"}), [3.0])
        encoded = base64.b64encode(b"page").decode("ascii")
        self.assertEqual(
            run_embedding(
                engine,
                {"input": {"source": {"kind": "page", "blob": "b"}, "bytes": f"data:image/png;base64,{encoded}"}},
            ),
            [4.0],
        )

    def test_response_preserves_model_and_embedding(self) -> None:
        body = vector_response("siglip", [0.0] * 768)
        payload = json.loads(body)
        self.assertEqual(payload["model"], "siglip")
        self.assertEqual(len(payload["data"][0]["embedding"]), 768)

    def test_response_rejects_wrong_dimensions(self) -> None:
        with self.assertRaises(ValueError):
            vector_response("siglip", [0.0])


if __name__ == "__main__":
    unittest.main()
