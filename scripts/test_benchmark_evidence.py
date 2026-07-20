from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "benchmark_evidence.py"
SPEC = importlib.util.spec_from_file_location("benchmark_evidence", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load benchmark_evidence.py")
EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EVIDENCE)
MANIFEST = ROOT / "tests" / "contracts" / "benchmark_evidence_v1.json"


class BenchmarkEvidenceManifestTests(unittest.TestCase):
    def test_checked_in_manifest_is_valid(self) -> None:
        self.assertEqual(EVIDENCE.errors_for_manifest(MANIFEST), [])

    def test_source_hash_drift_is_rejected(self) -> None:
        payload = json.loads(MANIFEST.read_text(encoding="utf-8"))
        payload["milestones"][0]["corpus"]["source_hash"] = "0" * 64
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            errors = EVIDENCE.errors_for_manifest(path)
        self.assertTrue(any("source_hash" in error for error in errors))

    def test_product_stage_requires_real_passing_measurements(self) -> None:
        payload = json.loads(MANIFEST.read_text(encoding="utf-8"))
        entry = payload["milestones"][0]
        entry["release_stage"] = "product-complete"
        entry["data_fidelity"] = "mixed"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            errors = EVIDENCE.errors_for_manifest(path)
        self.assertTrue(any("product stages require real" in error for error in errors))
        self.assertTrue(any("product stages require passing" in error for error in errors))

    def test_repository_report_contract_rejects_missing_measurement_status(self) -> None:
        report = {
            "measurement_kind": "real_repository_code_index",
            "evaluation_date": "2026-07-20",
            "corpus_id": "corpus-v1",
            "repository_revision": "commit-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "observations": [{"case_id": "case-1", "route": "PhaseC", "latency_ms": 1}],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "repository.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "repository")
        self.assertTrue(any("measurement_status" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
