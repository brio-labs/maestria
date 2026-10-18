"""Tests for the philosophy_check reporting family."""

from __future__ import annotations

import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest.mock import patch

from philosophy_check import reporting
from philosophy_check_testbase import PhilosophyCheckFixture


class ReportingTests(PhilosophyCheckFixture):

    def test_main_wires_all_scans_and_reports_each_violation_once(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "sillage-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            # Both kernel and domain scans flag the same forbidden token;
            # main() reports the violation once.
            source.write_text("std::fs::read(\"x\");\n", encoding="utf-8")

            captured = []
            original_print = print

            def spy_print(*args, **kwargs):
                captured.append(" ".join(str(arg) for arg in args))

            reporting.print = spy_print
            try:
                exit_code = reporting.main()
            finally:
                reporting.print = original_print

            self.assertNotEqual(exit_code, 0)
            output = "\n".join(captured)
            evidence_lines = [
                line
                for line in output.splitlines()
                if source.relative_to(root).as_posix() in line
                and "std::fs" in line
            ]
            self.assertEqual(len(evidence_lines), 1)

    def test_main_keeps_review_signals_non_blocking(self) -> None:
        review_signals = (
            "external/example/src/lib.rs has dense production modules",
            "external/example/src/orchestrator.rs has many child modules",
        )
        captured = []
        original_print = print

        def spy_print(*args, **kwargs):
            captured.append(" ".join(str(arg) for arg in args))

        reporting.print = spy_print
        try:
            with ExitStack() as stack:
                for name in dir(reporting):
                    if name.startswith("scan_"):
                        stack.enter_context(
                            patch.object(reporting, name, return_value=[])
                        )
                stack.enter_context(
                    patch.object(
                        reporting,
                        "scan_cohesion",
                        return_value=[review_signals[0]],
                    )
                )
                stack.enter_context(
                    patch.object(
                        reporting,
                        "scan_mixed_responsibilities",
                        return_value=[review_signals[1]],
                    )
                )
                exit_code = reporting.main()
        finally:
            reporting.print = original_print

        self.assertEqual(exit_code, 0)
        output = "\n".join(captured)
        self.assertIn(review_signals[0], output)
        self.assertIn(review_signals[1], output)