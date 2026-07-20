from __future__ import annotations

import importlib.util
import json
import re
import textwrap
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("release_exit_evidence.py")
SPEC = importlib.util.spec_from_file_location("release_exit_evidence", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load release_exit_evidence.py")
RELEASE_EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE_EVIDENCE)


class ReleaseExitEvidenceContractTests(unittest.TestCase):
    """Existing validation tests, kept and extended."""


    def test_release_workflow_uses_canonical_stage_choices(self) -> None:
        workflow = Path(__file__).resolve().parents[1] / ".github" / "workflows" / "release.yml"
        content = workflow.read_text(encoding="utf-8")
        match = re.search(
            r"(?ms)^      evidence_stage:\n(.*?)(?=^concurrency:)",
            content,
        )
        self.assertIsNotNone(match)
        assert match is not None
        options = re.findall(r"^\s+- ([a-z-]+)$", match.group(1), re.MULTILINE)
        # Planned is a tracking state, not a releasable workflow target.
        self.assertEqual(options, list(RELEASE_EVIDENCE.RELEASE_STATES[1:]))

    def _product_complete_payload(self, *, data_fidelity: str = "real") -> dict:
        return {

            "schema_version": 1,
            "release_stage": "product-complete",
            "benchmark": {
                "benchmark_date": "2026-07-19",
                "data_fidelity": data_fidelity,
                "fingerprints": {
                    "corpus_snapshot": "corpus-v1",
                    "index_generation": "idx-42",
                    "model_fingerprint": "provider:rerank-v3",
                },
                "results": {
                    "quality": {"status": "pass", "p50": 0.74},
                    "resource": {"status": "pass", "p95_latency_ms": 120},
                    "security": {"status": "pass", "violations": 0},
                },
                "degradations": [
                    {
                        "area": "query_class",
                        "status": "known",
                        "description": "table evidence is incomplete on scanned PDFs",
                    }
                ],
            },
            "post_release_work": [],
        }

    def _to_description(self, payload: dict) -> str:
        payload_json = json.dumps(payload, indent=2, sort_keys=True)
        return f"```release-exit-evidence\n{payload_json}\n```"

    def test_missing_exit_evidence_block_is_rejected(self) -> None:
        _, errors = RELEASE_EVIDENCE.parse_exit_evidence("No evidence block in this milestone.")
        self.assertTrue(any("No release-exit evidence block found." in error for error in errors))

    def test_invalid_json_exit_evidence_block_is_rejected(self) -> None:
        _, errors = RELEASE_EVIDENCE.parse_exit_evidence("```release-exit-evidence\n{bad json}\n```")
        self.assertTrue(any("not valid JSON" in error for error in errors))

    def test_synthetic_benchmark_with_product_complete_stage_is_rejected(self) -> None:
        payload = self._product_complete_payload(data_fidelity="synthetic")
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="product-complete")
        self.assertEqual(stage, "product-complete")
        self.assertTrue(any("`benchmark.data_fidelity` must be 'real'" in error for error in errors))

    def test_invalid_fingerprint_fields_rejected(self) -> None:
        payload = self._product_complete_payload()
        payload["benchmark"]["fingerprints"]["model_fingerprint"] = ""
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="product-complete")
        self.assertEqual(stage, "product-complete")
        self.assertTrue(any("fingerprints.model_fingerprint" in error for error in errors))

    def test_valid_product_complete_payload_passes(self) -> None:
        payload = self._product_complete_payload()
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="product-complete")
        self.assertEqual(stage, "product-complete")
        self.assertEqual(errors, [])

    def test_benchmark_complete_with_synthetic_data_does_not_mark_product_complete(self) -> None:
        payload = {
            "schema_version": 1, "release_stage": "benchmark-complete",
            "benchmark": {
                "benchmark_date": "2026-07-19", "data_fidelity": "synthetic",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": [],
            },
            "post_release_work": [{"group": "maintenance/release", "status": "open", "description": "Run real corpus benchmark"}],
        }
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="implementation-complete", require_maintenance_group=False)
        self.assertEqual(stage, "benchmark-complete")
        self.assertEqual(errors, [])

    def test_benchmark_complete_requires_maintenance_grouping_when_enforced(self) -> None:
        payload = {
            "schema_version": 1, "release_stage": "benchmark-complete",
            "benchmark": {
                "benchmark_date": "2026-07-19", "data_fidelity": "synthetic",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": [],
            },
            "post_release_work": [{"group": "other-group", "status": "open", "description": "Run real corpus benchmark"}],
        }
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="benchmark-complete", require_maintenance_group=True)
        self.assertEqual(stage, "benchmark-complete")
        self.assertTrue(any("missing a `maintenance/release` group entry" in error for error in errors))

    def test_released_stage_requires_post_release_work(self) -> None:
        payload = self._product_complete_payload()
        payload["release_stage"] = "released"
        payload["post_release_work"] = []
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="released")
        self.assertEqual(stage, "released")
        self.assertTrue(any("`post_release_work` is required" in error for error in errors))

    def test_released_stage_with_post_release_work_passes(self) -> None:
        payload = self._product_complete_payload()
        payload["release_stage"] = "released"
        payload["post_release_work"] = [{"group": "maintenance/release", "status": "done", "issue": "https://github.com/brio-labs/maestria/issues/9999"}]
        stage, errors = RELEASE_EVIDENCE.validate_exit_evidence(payload, required_stage="released")
        self.assertEqual(stage, "released")
        self.assertEqual(errors, [])

    def test_description_extracted_from_release_evidence_block(self) -> None:
        payload = self._product_complete_payload()
        parsed_payload, parse_errors = RELEASE_EVIDENCE.parse_exit_evidence(self._to_description(payload))
        self.assertEqual(parse_errors, [])
        self.assertIsInstance(parsed_payload, dict)
        stage, validation_errors = RELEASE_EVIDENCE.validate_exit_evidence(parsed_payload, required_stage="product-complete")
        self.assertEqual(stage, "product-complete")
        self.assertEqual(validation_errors, [])


class ExitEvidenceGenerationTests(unittest.TestCase):
    """Tests for generate_exit_evidence and generate_exit_evidence_block."""

    def test_generate_minimal_implementation_complete(self) -> None:
        payload = RELEASE_EVIDENCE.generate_exit_evidence(release_stage="implementation-complete")
        self.assertEqual(payload["schema_version"], 1)
        self.assertEqual(payload["release_stage"], "implementation-complete")
        self.assertNotIn("benchmark", payload)

    def test_generate_with_full_data(self) -> None:
        payload = RELEASE_EVIDENCE.generate_exit_evidence(
            release_stage="product-complete", benchmark_date="2026-07-19", data_fidelity="real",
            fingerprints={"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
            results={"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
            degradations=[{"area": "query_class", "status": "known", "description": "table evidence is incomplete on scanned PDFs"}],
            post_release_work=[{"group": "maintenance/release", "status": "done", "issue": "#9999"}],
            environment={"os": "ubuntu-24.04", "rust_toolchain": "stable", "cpu_arch": "x86_64"},
        )
        self.assertIn("benchmark", payload)
        self.assertIn("environment", payload["benchmark"])
        self.assertIn("post_release_work", payload)
        self.assertEqual(payload["benchmark"]["environment"]["os"], "ubuntu-24.04")

    def test_generate_block_creates_fence(self) -> None:
        block = RELEASE_EVIDENCE.generate_exit_evidence_block(release_stage="benchmark-complete", benchmark_date="2026-07-20")
        self.assertTrue(block.startswith("```release-exit-evidence\n"))
        self.assertTrue(block.endswith("```\n"))
        parsed, errors = RELEASE_EVIDENCE.parse_exit_evidence(block)
        self.assertEqual(errors, [])
        self.assertEqual(parsed["release_stage"], "benchmark-complete")

    def test_generate_block_json_fence(self) -> None:
        block = RELEASE_EVIDENCE.generate_exit_evidence_block(release_stage="implementation-complete", fence="json")
        self.assertTrue(block.startswith("```json\n"))

    def test_generate_block_invalid_fence_falls_back(self) -> None:
        block = RELEASE_EVIDENCE.generate_exit_evidence_block(release_stage="implementation-complete", fence="no-such-fence")
        self.assertTrue(block.startswith("```release-exit-evidence\n"))

    def test_generate_profiles(self) -> None:
        payload = RELEASE_EVIDENCE.generate_exit_evidence(release_stage="benchmark-complete", benchmark_date="2026-07-20",
            profiles={"version": 1, "entries": [{"stage": "baseline", "name": "corpus-v1-baseline"}, {"stage": "golden", "name": "corpus-v1-golden"}]})
        self.assertIn("profiles", payload)
        self.assertEqual(payload["profiles"]["version"], 1)

    def test_generate_artifacts(self) -> None:
        payload = RELEASE_EVIDENCE.generate_exit_evidence(release_stage="product-complete", benchmark_date="2026-07-20",
            artifacts=[{"source": "ci", "url": "https://github.com/example/repo/actions/runs/1", "label": "CI run #1"}])
        self.assertIn("artifacts", payload["benchmark"])
        self.assertEqual(payload["benchmark"]["artifacts"][0]["source"], "ci")


class EnvironmentValidationTests(unittest.TestCase):
    """Tests for environment and artifact validation blocks."""

    def _base_payload(self) -> dict:
        return {
            "schema_version": 1, "release_stage": "product-complete",
            "benchmark": {
                "benchmark_date": "2026-07-19", "data_fidelity": "real",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": [],
            },
            "post_release_work": [],
        }

    def test_valid_environment_passes(self) -> None:
        p = self._base_payload()
        p["benchmark"]["environment"] = {"os": "ubuntu-24.04", "rust_toolchain": "stable", "cpu_arch": "x86_64"}
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p)
        self.assertEqual(errors, [])

    def test_environment_missing_keys_rejected(self) -> None:
        p = self._base_payload()
        p["benchmark"]["environment"] = {"os": "ubuntu-24.04"}
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p)
        self.assertTrue(any("environment.rust_toolchain" in e for e in errors))
        self.assertTrue(any("environment.cpu_arch" in e for e in errors))

    def test_environment_is_optional(self) -> None:
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(self._base_payload())
        self.assertEqual(errors, [])

    def test_valid_artifacts_passes(self) -> None:
        p = self._base_payload()
        p["benchmark"]["artifacts"] = [{"source": "ci", "url": "https://example.com/run/123", "label": "CI run #123"}]
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p)
        self.assertEqual(errors, [])

    def test_artifact_invalid_source_rejected(self) -> None:
        p = self._base_payload()
        p["benchmark"]["artifacts"] = [{"source": "invalid-source", "url": "https://example.com", "label": "bad"}]
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p)
        self.assertTrue(any("artifacts[0].source" in e for e in errors))

    def test_artifact_invalid_type_rejected(self) -> None:
        p = self._base_payload()
        p["benchmark"]["artifacts"] = "not-a-list"
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p)
        self.assertTrue(any("artifacts" in e for e in errors))


class GoldenProfileValidationTests(unittest.TestCase):
    """Tests for profiles (GoldenProfile) validation."""

    def _valid_payload(self, rs: str = "benchmark-complete") -> dict:
        return {
            "schema_version": 1, "release_stage": rs,
            "benchmark": {
                "benchmark_date": "2026-07-19", "data_fidelity": "real",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": [],
            },
            "post_release_work": [{"group": "maintenance/release", "status": "open", "description": "Follow-up"}],
        }

    def test_valid_profiles_passes(self) -> None:
        p = self._valid_payload()
        p["profiles"] = {"version": 1, "entries": [{"stage": "baseline", "name": "v1-baseline"}, {"stage": "golden", "name": "v1-golden"}]}
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="benchmark-complete")
        self.assertEqual(errors, [])

    def test_profiles_invalid_stage_rejected(self) -> None:
        p = self._valid_payload()
        p["profiles"] = {"version": 1, "entries": [{"stage": "invalid-stage", "name": "v1-test"}]}
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="benchmark-complete")
        self.assertTrue(any("profiles.entries[0].stage" in e for e in errors))

    def test_profiles_optional(self) -> None:
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence({"schema_version": 1, "release_stage": "implementation-complete"}, required_stage="implementation-complete")
        self.assertEqual(errors, [])

    def test_profiles_empty_entries_allowed(self) -> None:
        p = {"schema_version": 1, "release_stage": "implementation-complete", "profiles": {"version": 1, "entries": []}}
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="implementation-complete")
        self.assertEqual(errors, [])

    def test_profiles_wrong_version_rejected(self) -> None:
        p = {"schema_version": 1, "release_stage": "implementation-complete", "profiles": {"version": 999, "entries": []}}
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="implementation-complete")
        self.assertTrue(any("profiles.version" in e for e in errors))


class ReconciliationTests(unittest.TestCase):
    """Tests for reconcile_exit_evidence."""

    def test_reconcile_passes_on_match(self) -> None:
        ev = {"schema_version": 1, "release_stage": "product-complete", "benchmark": {"results": {"quality": {"p50": 0.74}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}}}}
        issues = RELEASE_EVIDENCE.reconcile_exit_evidence(evidence_payload=ev, actual_results={"quality": {"p50": 0.74}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}})
        self.assertEqual(issues, [])

    def test_reconcile_detects_mismatch(self) -> None:
        ev = {"schema_version": 1, "release_stage": "product-complete", "benchmark": {"results": {"quality": {"p50": 0.74}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}}}}
        issues = RELEASE_EVIDENCE.reconcile_exit_evidence(evidence_payload=ev, actual_results={"quality": {"p50": 0.62}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}})
        self.assertEqual(len([i for i in issues if i["kind"] == "mismatch"]), 1)

    def test_reconcile_detects_missing_evidence_field(self) -> None:
        ev = {"schema_version": 1, "release_stage": "product-complete", "benchmark": {"results": {"quality": {"status": "pass"}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}}}}
        issues = RELEASE_EVIDENCE.reconcile_exit_evidence(evidence_payload=ev, actual_results={"quality": {"p50": 0.74}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}})
        self.assertTrue(any("quality.p50" in i["field"] for i in issues))

    def test_reconcile_environment_drift(self) -> None:
        ev = {"schema_version": 1, "release_stage": "product-complete", "benchmark": {"results": {"quality": {"p50": 0.74}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}},
            "environment": {"os": "ubuntu-22.04", "rust_toolchain": "stable", "cpu_arch": "x86_64"}}}
        issues = RELEASE_EVIDENCE.reconcile_exit_evidence(evidence_payload=ev, actual_environments={"os": "ubuntu-24.04", "rust_toolchain": "stable", "cpu_arch": "aarch64"})
        self.assertEqual(len([i for i in issues if i["kind"] == "env_drift"]), 2)

    def test_reconcile_environment_missing(self) -> None:
        ev = {"schema_version": 1, "release_stage": "product-complete", "benchmark": {"results": {"quality": {"p50": 0.74}, "resource": {"p95_latency_ms": 120}, "security": {"violations": 0}}}}
        issues = RELEASE_EVIDENCE.reconcile_exit_evidence(evidence_payload=ev, actual_environments={"os": "ubuntu-24.04", "rust_toolchain": "stable", "cpu_arch": "x86_64"})
        self.assertEqual(len([i for i in issues if i["kind"] == "env_missing"]), 3)

    def test_reconcile_no_benchmark_section(self) -> None:
        issues = RELEASE_EVIDENCE.reconcile_exit_evidence(evidence_payload={"schema_version": 1, "release_stage": "implementation-complete"}, actual_results={"quality": {"p50": 0.74}})
        self.assertIsInstance(issues, list)


class PostReleaseTrackingValidationTests(unittest.TestCase):
    """Tests for validate_post_release_tracking."""

    def test_empty_work_items_passes(self) -> None:
        self.assertTrue(RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[])[0])

    def test_none_work_items_passes(self) -> None:
        self.assertTrue(RELEASE_EVIDENCE.validate_post_release_tracking(work_items=None)[0])

    def test_valid_work_items_passes(self) -> None:
        ok, msgs = RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[{"group": "maint", "status": "done", "issue": "#1"}, {"group": "migrate", "status": "open"}])
        self.assertTrue(ok)

    def test_done_item_missing_issue_reported(self) -> None:
        ok, msgs = RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[{"group": "maint", "status": "done"}])
        self.assertFalse(ok)
        self.assertTrue(any("no `issue` URL" in m for m in msgs))

    def test_invalid_status_reported(self) -> None:
        ok, msgs = RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[{"group": "t", "status": "bad-status"}])
        self.assertFalse(ok)
        self.assertTrue(any("status" in m for m in msgs))

    def test_missing_group_reported(self) -> None:
        ok, msgs = RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[{"status": "open"}])
        self.assertFalse(ok)
        self.assertTrue(any("missing a `group`" in m for m in msgs))

    def test_follow_up_issues_checked(self) -> None:
        ok, _ = RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[{"group": "maint", "status": "open"}], follow_up_issues={"maint": "https://github.com/x/y/issues/1"})
        self.assertTrue(ok)

    def test_follow_up_issues_missing_reported(self) -> None:
        ok, msgs = RELEASE_EVIDENCE.validate_post_release_tracking(work_items=[{"group": "maint", "status": "open"}], follow_up_issues={})
        self.assertFalse(ok)
        self.assertTrue(any("missing" in m for m in msgs))


class StagedWorkflowTests(unittest.TestCase):
    """Tests for staged data_fidelity handling."""

    def test_benchmark_complete_with_staged_data_passes(self) -> None:
        p = {
            "schema_version": 1, "release_stage": "benchmark-complete",
            "benchmark": {"benchmark_date": "2026-07-19", "data_fidelity": "staged",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": []},
            "post_release_work": [{"group": "maintenance/release", "status": "open", "description": "Real benchmark"}],
        }
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="benchmark-complete")
        self.assertEqual(errors, [])

    def test_product_complete_with_staged_data_rejected(self) -> None:
        p = {
            "schema_version": 1, "release_stage": "product-complete",
            "benchmark": {"benchmark_date": "2026-07-19", "data_fidelity": "staged",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": []},
            "post_release_work": [{"group": "maintenance/release", "status": "open", "description": "Real benchmark"}],
        }
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="product-complete")
        self.assertTrue(any("data_fidelity" in e for e in errors))

    def test_stage_progression_validation(self) -> None:
        p = {
            "schema_version": 1, "release_stage": "benchmark-complete",
            "benchmark": {"benchmark_date": "2026-07-19", "data_fidelity": "real",
                "fingerprints": {"corpus_snapshot": "corpus-v1", "index_generation": "idx-42", "model_fingerprint": "provider:rerank-v3"},
                "results": {"quality": {"status": "pass", "p50": 0.74}, "resource": {"status": "pass", "p95_latency_ms": 120}, "security": {"status": "pass", "violations": 0}},
                "degradations": []},
            "post_release_work": [],
        }
        _, errors = RELEASE_EVIDENCE.validate_exit_evidence(p, required_stage="product-complete")
        self.assertTrue(any("preflight requires at least" in e for e in errors))


class CLITests(unittest.TestCase):
    """Tests for CLI entry points (subcommand dispatch)."""

    def test_parser_has_validate(self) -> None:
        parser = RELEASE_EVIDENCE.build_parser()
        args = parser.parse_args(["validate", "--description-file", "/dev/null", "--required-stage", "product-complete"])
        self.assertEqual(args.command, "validate")

    def test_parser_has_generate(self) -> None:
        parser = RELEASE_EVIDENCE.build_parser()
        args = parser.parse_args(["generate", "--release-stage", "implementation-complete"])
        self.assertEqual(args.command, "generate")

    def test_parser_has_reconcile(self) -> None:
        parser = RELEASE_EVIDENCE.build_parser()
        args = parser.parse_args(["reconcile", "--evidence-file", "/dev/null"])
        self.assertEqual(args.command, "reconcile")

    def test_parser_has_validate_tracking(self) -> None:
        parser = RELEASE_EVIDENCE.build_parser()
        args = parser.parse_args(["validate-tracking", "--work-items-file", "/dev/null"])
        self.assertEqual(args.command, "validate-tracking")


if __name__ == "__main__":
    unittest.main()
