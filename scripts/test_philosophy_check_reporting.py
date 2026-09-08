"""Tests for the philosophy_check reporting family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import reporting, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class ReportingTests(PhilosophyCheckFixture):

    def test_main_wires_all_scans_and_reports_each_violation_once(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
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
            self.assertEqual(
                output.count("contains forbidden kernel token std::fs"),
                1,
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs",
                output,
            )
