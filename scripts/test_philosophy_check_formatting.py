"""Tests for the philosophy_check formatting family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import dependency_graph, formatting, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class FormattingTests(PhilosophyCheckFixture):

    def test_scan_readability_rejects_width_and_dense_statements(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "fn example() {\n"
                "    let first = 1; let second = 2;\n"
                f"    let long_name = {'x' * 120};\n"
                "}\n",
                encoding="utf-8",
            )

            violations = formatting.scan_readability_style()
            self.assertTrue(any("production code line is" in item for item in violations), violations)
            self.assertTrue(any("multiple production statements" in item for item in violations), violations)

    def test_scan_readability_allows_simple_declarations(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "struct Pair { left: u8 }\n"
                "fn empty() {}\n"
                "fn compact() { 1 + 1 }\n",
                encoding="utf-8",
            )

            violations = formatting.scan_readability_style()
            self.assertEqual(len(violations), 1, violations)
            self.assertIn("one-line production function body", violations[0])

    def test_exemption_expiry_is_enforced_at_target_version(self) -> None:
        old_module = formatting.MODULE_SIZE_EXEMPTIONS
        old_adr = formatting.ADR_MODULE_EXEMPTIONS
        old_fn = formatting.FUNCTION_SIZE_EXEMPTIONS
        old_mixed = formatting.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            formatting.MODULE_SIZE_EXEMPTIONS = {
                "crates/example/src/large.rs": "2026-12-31",
            }
            formatting.ADR_MODULE_EXEMPTIONS = {}
            formatting.FUNCTION_SIZE_EXEMPTIONS = {}
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = {}
            self.assertEqual(formatting.scan_exemption_expiry("2026-06-01"), [])
            self.assertEqual(len(formatting.scan_exemption_expiry("2026-12-31")), 1)
            self.assertEqual(len(formatting.scan_exemption_expiry("2027-01-01")), 1)
        finally:
            formatting.MODULE_SIZE_EXEMPTIONS = old_module
            formatting.ADR_MODULE_EXEMPTIONS = old_adr
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_fn
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed

    def test_exemption_expiry_rejects_malformed_target(self) -> None:
        old_module = formatting.MODULE_SIZE_EXEMPTIONS
        old_adr = formatting.ADR_MODULE_EXEMPTIONS
        old_fn = formatting.FUNCTION_SIZE_EXEMPTIONS
        old_mixed = formatting.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            formatting.MODULE_SIZE_EXEMPTIONS = {
                "crates/example/src/large.rs": "2026-07",
            }
            formatting.ADR_MODULE_EXEMPTIONS = {}
            formatting.FUNCTION_SIZE_EXEMPTIONS = {}
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = {}
            violations = formatting.scan_exemption_expiry("2026-06-01")
            self.assertEqual(len(violations), 1)
            self.assertIn("malformed", violations[0])
        finally:
            formatting.MODULE_SIZE_EXEMPTIONS = old_module
            formatting.ADR_MODULE_EXEMPTIONS = old_adr
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_fn
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed

    def test_module_size_scan_reports_unexempt_large_module(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "core" / "maestria-core" / "src" / "large.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "\n".join(f"pub fn item_{index}() {{}}" for index in range(401)),
                encoding="utf-8",
            )

            violations = formatting.scan_module_sizes()

            self.assertEqual(
                violations,
                [
                    "crates/core/maestria-core/src/large.rs has "
                    "401 module logical lines (limit 400)"
                ],
            )

    def test_module_size_scan_reports_oversized_test_file_physical_budget(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "core" / "maestria-core" / "tests" / "large.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "\n".join("fn test_case() {}" for _ in range(901)), encoding="utf-8"
            )

            violations = formatting.scan_module_sizes()

            self.assertEqual(
                violations,
                [
                    "crates/core/maestria-core/tests/large.rs has "
                    "901 physical lines (limit 900)"
                ],
            )

    def test_facade_boundary_honors_existing_adr_exemption_only_for_named_path(self) -> None:
        old_exemptions = formatting.ADR_MODULE_EXEMPTIONS
        try:
            formatting.ADR_MODULE_EXEMPTIONS = {
                "crates/runtime/example/src/lib.rs": "v9.0.0",
            }
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.configure_root(root)
                lib_dir = root / "crates" / "runtime" / "example" / "src"
                lib_dir.mkdir(parents=True)
                (lib_dir / "lib.rs").write_text(
                    "pub fn reviewed_legacy_body() {}\n", encoding="utf-8"
                )

                self.assertEqual(len(dependency_graph.scan_facade_boundaries()), 1)
        finally:
            formatting.ADR_MODULE_EXEMPTIONS = old_exemptions

    def test_scan_function_sizes_reports_oversized_function(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "logic.rs"
            source.parent.mkdir(parents=True)
            body = "\n".join(f"    let _ = {i};" for i in range(101))
            source.write_text(f"pub fn big() {{\n{body}\n}}\n", encoding="utf-8")

            violations = formatting.scan_function_sizes()
            self.assertEqual(len(violations), 1)
            self.assertIn("function `big` has 101 logical lines", violations[0])

    def test_scan_function_sizes_skips_test_sources(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "tests" / "integration.rs"
            source.parent.mkdir(parents=True)
            body = "\n".join(f"    let _ = {i};" for i in range(100))
            source.write_text(f"fn big() {{\n{body}\n}}\n", encoding="utf-8")

            self.assertEqual(formatting.scan_function_sizes(), [])

    def test_scan_function_sizes_respects_exemptions(self) -> None:
        old_exemptions = formatting.FUNCTION_SIZE_EXEMPTIONS
        try:
            formatting.FUNCTION_SIZE_EXEMPTIONS = {
                "crates/apps/example/src/logic.rs": {"big": "v0.9.0"},
            }
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.configure_root(root)
                source = root / "crates" / "apps" / "example" / "src" / "logic.rs"
                source.parent.mkdir(parents=True)
                body = "\n".join(f"    let _ = {i};" for i in range(100))
                source.write_text(f"pub fn big() {{\n{body}\n}}\n", encoding="utf-8")

                self.assertEqual(formatting.scan_function_sizes(), [])
        finally:
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_exemptions

    def test_function_exemption_is_scoped_to_named_item(self) -> None:
        old_exemptions = formatting.FUNCTION_SIZE_EXEMPTIONS
        try:
            formatting.FUNCTION_SIZE_EXEMPTIONS = {
                "crates/apps/example/src/logic.rs": {"known": "v0.9.0"},
            }
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.configure_root(root)
                source = root / "crates" / "apps" / "example" / "src" / "logic.rs"
                source.parent.mkdir(parents=True)
                body = "\n".join(f"    let _ = {i};" for i in range(101))
                source.write_text(
                    f"pub fn known() {{\n{body}\n}}\n"
                    f"pub fn newly_added() {{\n{body}\n}}\n",
                    encoding="utf-8",
                )

                violations = formatting.scan_function_sizes()

                self.assertEqual(len(violations), 1)
                self.assertIn("function `newly_added`", violations[0])
        finally:
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_exemptions

    def test_scan_mixed_responsibilities_flags_large_multi_mod_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "orchestrator.rs"
            source.parent.mkdir(parents=True)
            lines = ["mod a;", "mod b;", "mod c;"]
            lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
            source.write_text("\n".join(lines) + "\n", encoding="utf-8")

            violations = formatting.scan_mixed_responsibilities()
            self.assertEqual(len(violations), 1)
            self.assertIn("mixed-responsibility signal", violations[0])

    def test_scan_mixed_responsibilities_skips_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            lines = ["mod a;", "mod b;", "mod c;"]
            lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
            source.write_text("\n".join(lines) + "\n", encoding="utf-8")

            self.assertEqual(formatting.scan_mixed_responsibilities(), [])

    def test_scan_mixed_responsibilities_respects_exemptions(self) -> None:
        old_exemptions = formatting.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = {
                "crates/apps/example/src/orchestrator.rs": "v0.9.0",
            }
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.configure_root(root)
                source = (
                    root / "crates" / "apps" / "example" / "src" / "orchestrator.rs"
                )
                source.parent.mkdir(parents=True)
                lines = ["mod a;", "mod b;", "mod c;"]
                lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
                source.write_text("\n".join(lines) + "\n", encoding="utf-8")

                self.assertEqual(formatting.scan_mixed_responsibilities(), [])
        finally:
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = old_exemptions

    def test_exemption_expiry_covers_function_and_mixed_responsibility_exemptions(
        self,
    ) -> None:
        old_fn = formatting.FUNCTION_SIZE_EXEMPTIONS
        old_mixed = formatting.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            formatting.FUNCTION_SIZE_EXEMPTIONS = {
                "crates/example/src/large_fn.rs": {"large": "2026-06-01"},
            }
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = {
                "crates/example/src/mixed.rs": "2026-06-01",
            }
            formatting.MODULE_SIZE_EXEMPTIONS = {}
            formatting.ADR_MODULE_EXEMPTIONS = {}
            violations = formatting.scan_exemption_expiry("2026-07-01")
            self.assertEqual(len(violations), 2)
            paths = {v.split()[0] for v in violations}
            self.assertEqual(
                paths,
                {
                    "crates/example/src/large_fn.rs::large",
                    "crates/example/src/mixed.rs",
                },
            )
        finally:
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_fn
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed
