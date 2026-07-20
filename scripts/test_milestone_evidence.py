from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "release_exit_evidence.py"
SPEC = importlib.util.spec_from_file_location("release_exit_evidence", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load release_exit_evidence.py")
RELEASE_EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RELEASE_EVIDENCE)

FIXTURE = (
    Path(__file__).resolve().parent.parent
    / "tests"
    / "contracts"
    / "milestone_evidence_v0.4_v0.9.json"
)


class MilestoneEvidenceFixtureTests(unittest.TestCase):
    """Validate every entry in the checked-in milestone evidence manifest
    against the release-exit-evidence contract."""

    maxDiff = None

    def setUp(self) -> None:
        self.raw = json.loads(FIXTURE.read_text(encoding="utf-8"))
        self.milestones = self.raw["milestones"]

    def test_manifest_has_expected_milestones(self) -> None:
        titles = [m["milestone"] for m in self.milestones]
        expected = [
            "v0.4 — Deterministic Search Baseline",
            "v0.5 — Evaluated Hybrid Retrieval",
            "v0.6 — Query-Adaptive Search",
            "v0.7 — Repository Intelligence",
            "v0.8 — Visual Document Retrieval",
            "v0.9 — Advanced Retrieval Research",
        ]
        self.assertEqual(titles, expected)

    def test_every_stage_is_valid_release_state(self) -> None:
        for entry in self.milestones:
            stage = entry["release_stage"]
            self.assertIn(
                stage,
                RELEASE_EVIDENCE.RELEASE_STATES,
                f"{entry['milestone']}: {stage!r} is not a valid release stage",
            )

    def test_every_description_block_parses_and_validates(self) -> None:
        for entry in self.milestones:
            desc = entry["description_block"]
            payload, parse_errors = RELEASE_EVIDENCE.parse_exit_evidence(desc)
            self.assertEqual(
                parse_errors,
                [],
                f"{entry['milestone']}: parse errors: {parse_errors}",
            )
            self.assertIsNotNone(payload, f"{entry['milestone']}: payload must not be None")
            assert payload is not None

            stage, validation_errors = RELEASE_EVIDENCE.validate_exit_evidence(
                payload,
                required_stage=entry["release_stage"],
            )
            self.assertEqual(
                validation_errors,
                [],
                f"{entry['milestone']}: validation errors: {validation_errors}",
            )
            self.assertEqual(
                stage,
                entry["release_stage"],
                f"{entry['milestone']}: stage mismatch",
            )

    def test_closable_milestones_have_implementation_complete(self) -> None:
        for entry in self.milestones:
            if entry["closure"] == "closable":
                self.assertEqual(
                    entry["release_stage"],
                    "implementation-complete",
                    f"{entry['milestone']}: closable milestone must be implementation-complete",
                )

    def test_open_milestones_have_known_stage(self) -> None:
        for entry in self.milestones:
            if entry["closure"] == "open":
                self.assertIn(
                    entry["release_stage"],
                    {"planned", "implementation-complete"},
                    f"{entry['milestone']}: open milestone has unexpected stage",
                )

    def test_v0_9_is_planned_while_research_issues_remain_open(self) -> None:
        v0_9 = next(m for m in self.milestones if m["milestone"].startswith("v0.9 — "))
        self.assertEqual(v0_9["release_stage"], "planned")
        self.assertEqual(v0_9["closure"], "open")
        self.assertIn("#90–#95", v0_9["summary"])

    def test_v0_6_records_published_release(self) -> None:
        v0_6 = next(m for m in self.milestones if m["milestone"].startswith("v0.6 — "))
        self.assertEqual(v0_6["published_release"], "v0.6.1")


if __name__ == "__main__":
    unittest.main()
