from __future__ import annotations

import base64
import json
import importlib.util
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("rapidocr_server.py")
SPEC = importlib.util.spec_from_file_location("rapidocr_server", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load rapidocr_server.py")
RAPIDOCR_SERVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RAPIDOCR_SERVER)

completion_response = RAPIDOCR_SERVER.completion_response
image_data_from_request = RAPIDOCR_SERVER.image_data_from_request
ocr_text = RAPIDOCR_SERVER.ocr_text


class RapidOcrServerTests(unittest.TestCase):
    def test_extracts_image_data_url_from_chat_request(self) -> None:
        image = b"png-bytes"
        payload = {
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {"type": "text", "text": "document parsing."},
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": "data:image/png;base64,"
                                + base64.b64encode(image).decode("ascii")
                            },
                        },
                    ],
                }
            ]
        }
        self.assertEqual(image_data_from_request(payload), image)

    def test_flattens_rapidocr_result_without_coordinates(self) -> None:
        result = [
            ([[0, 0], [1, 0], [1, 1], [0, 1]], " first ", 0.99),
            ([[0, 2], [1, 2], [1, 3], [0, 3]], "second", 0.98),
        ]
        self.assertEqual(ocr_text(result), "first\nsecond")

    def test_returns_openai_compatible_completion(self) -> None:
        response = json.loads(completion_response("rapidocr-test", "recognized"))
        self.assertEqual(response["model"], "rapidocr-test")
        self.assertEqual(response["choices"][0]["message"]["content"], "recognized")


if __name__ == "__main__":
    unittest.main()
