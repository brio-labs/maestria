"""Tests for the philosophy_check formatting family."""

from __future__ import annotations

import tempfile
from pathlib import Path

from philosophy_check import dependency_graph, formatting
from philosophy_check_testbase import PhilosophyCheckFixture


class FormattingTests(PhilosophyCheckFixture):

    def write_workspace_package(
        self,
        root: Path,
        library_source: str,
        package_path: str = "crates/core/sillage-core",
    ) -> tuple[Path, Path]:
        (root / "Cargo.toml").write_text(
            f'[workspace]\nmembers = ["{package_path}"]\n',
            encoding="utf-8",
        )
        crate_root = root / package_path
        source_dir = crate_root / "src"
        source_dir.mkdir(parents=True)
        (crate_root / "Cargo.toml").write_text(
            f'[package]\nname = "{crate_root.name}"\n'
            'version = "0.1.0"\nedition = "2021"\n',
            encoding="utf-8",
        )
        (source_dir / "lib.rs").write_text(library_source, encoding="utf-8")
        return crate_root, source_dir

    def test_scan_readability_rejects_width_and_dense_statements(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root,
                "fn example() {\n"
                "    let first = 1; let second = 2;\n"
                f"    let long_name = {'x' * 120};\n"
                "}\n",
                "crates/apps/example",
            )
            source = source_dir / "lib.rs"

            violations = formatting.scan_readability_style()
            relative_path = source.relative_to(root).as_posix()
            self.assertEqual(len(violations), 2, violations)
            self.assertTrue(any(f"{relative_path}:2" in item for item in violations))
            self.assertTrue(any(f"{relative_path}:3" in item for item in violations))

    def test_scan_readability_flags_compact_executable_function_body(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root,
                "struct Pair { left: u8 }\n"
                "fn empty() {}\n"
                "fn compact() { 1 + 1 }\n",
                "crates/apps/example",
            )
            source = source_dir / "lib.rs"

            violations = formatting.scan_readability_style()
            self.assertEqual(len(violations), 1, violations)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:3", violations[0]
            )

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

    def test_exemption_expiry_rejects_invalid_date(self) -> None:
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
            self.assertIn("crates/example/src/large.rs", violations[0])
        finally:
            formatting.MODULE_SIZE_EXEMPTIONS = old_module
            formatting.ADR_MODULE_EXEMPTIONS = old_adr
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_fn
            formatting.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed

    def test_module_size_scan_reports_unexempt_large_module(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root, "pub mod large;\n"
            )
            source = source_dir / "large.rs"
            source.write_text(
                "\n".join(f"pub fn item_{index}() {{}}" for index in range(401)),
                encoding="utf-8",
            )

            violations = formatting.scan_module_sizes()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])
            self.assertIn("401", violations[0])
            self.assertIn("400", violations[0])

    def test_module_size_scan_accepts_exact_logical_and_physical_budgets(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root, "pub mod large;\n"
            )
            source = source_dir / "large.rs"
            lines = [f"pub fn item_{index}() {{}}" for index in range(400)]
            lines.extend("// physical padding" for _ in range(500))
            source.write_text("\n".join(lines), encoding="utf-8")

            self.assertEqual(len(lines), 900)
            self.assertEqual(formatting.scan_module_sizes(), [])

    def test_module_size_scan_masks_public_inner_test_scope_at_exact_budget(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root, "pub mod large;\n"
            )
            source = source_dir / "large.rs"
            lines = [f"pub fn item_{index}() {{}}" for index in range(400)]
            lines.extend(
                (
                    "pub mod inner {",
                    "    #![cfg(test)]",
                    "    pub fn test_only() {}",
                    "}",
                )
            )
            source.write_text("\n".join(lines), encoding="utf-8")

            self.assertEqual(formatting.scan_module_sizes(), [])
            attributed_scope = (
                '#[doc = "test module"]\n'
                "pub mod inner {\n"
                "    #![cfg(test)]\n"
                "    pub fn test_only() {}\n"
                "}\n"
                "pub fn shipping() {}\n"
            )
            masked_scope = formatting.production_rust(attributed_scope)
            self.assertNotIn("#[doc", masked_scope)
            self.assertNotIn("pub mod inner", masked_scope)
            self.assertIn("pub fn shipping", masked_scope)
            self.assertEqual(
                masked_scope.count("\n"), attributed_scope.count("\n")
            )

    def test_module_size_scan_excludes_inline_test_code_from_logical_budget(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            test_functions = "\n".join(
                f"    fn case_{index}() {{}}" for index in range(401)
            )
            _crate_root, source_dir = self.write_workspace_package(
                root,
                "#[cfg(test)]\nmod tests {\n"
                f"{test_functions}\n"
                "}\n",
            )

            self.assertEqual(
                formatting.scan_module_sizes(),
                [],
            )

    def test_module_size_scan_enforces_physical_budget_for_cargo_test_target(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            crate_root, _source_dir = self.write_workspace_package(
                root, "pub fn helper() {}\n"
            )
            source = crate_root / "tests" / "large.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "\n".join(f"fn test_case_{index}() {{}}" for index in range(900)),
                encoding="utf-8",
            )

            self.assertEqual(formatting.scan_module_sizes(), [])

            source.write_text(
                "\n".join(f"fn test_case_{index}() {{}}" for index in range(901)),
                encoding="utf-8",
            )
            violations = formatting.scan_module_sizes()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])
            self.assertIn("901", violations[0])
            self.assertIn("900", violations[0])

    def test_facade_boundary_scans_workspace_library_target(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root,
                "pub fn reviewed_legacy_body() {}\n",
                "crates/runtime/example",
            )
            source = source_dir / "lib.rs"

            violations = dependency_graph.scan_facade_boundaries()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])

    def test_scan_function_sizes_reports_oversized_function(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root, "pub mod logic;\n", "crates/apps/example"
            )
            source = source_dir / "logic.rs"
            body = "\n".join(f"    let _ = {i};" for i in range(101))
            source.write_text(f"pub fn big() {{\n{body}\n}}\n", encoding="utf-8")

            violations = formatting.scan_function_sizes()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])
            self.assertIn("101", violations[0])
            self.assertIn("100", violations[0])

    def test_scan_function_sizes_accepts_exact_logical_budget(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root, "pub mod logic;\n", "crates/apps/example"
            )
            source = source_dir / "logic.rs"
            body = "\n".join(f"    let _ = {i};" for i in range(100))
            source.write_text(f"pub fn big() {{\n{body}\n}}\n", encoding="utf-8")

            self.assertEqual(formatting.scan_function_sizes(), [])

    def test_scan_function_sizes_skips_actual_cargo_test_sources(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            crate_root, _source_dir = self.write_workspace_package(
                root, "pub fn helper() {}\n", "crates/apps/example"
            )
            source = crate_root / "tests" / "integration.rs"
            source.parent.mkdir(parents=True)
            body = "\n".join(f"    let _ = {i};" for i in range(101))
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
                _crate_root, source_dir = self.write_workspace_package(
                    root, "pub mod logic;\n", "crates/apps/example"
                )
                source = source_dir / "logic.rs"
                body = "\n".join(f"    let _ = {i};" for i in range(101))
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
                _crate_root, source_dir = self.write_workspace_package(
                    root, "pub mod logic;\n", "crates/apps/example"
                )
                source = source_dir / "logic.rs"
                body = "\n".join(f"    let _ = {i};" for i in range(101))
                source.write_text(
                    f"pub fn known() {{\n{body}\n}}\n"
                    f"pub fn newly_added() {{\n{body}\n}}\n",
                    encoding="utf-8",
                )

                violations = formatting.scan_function_sizes()
                self.assertEqual(len(violations), 1)
                self.assertIn(source.relative_to(root).as_posix(), violations[0])
                self.assertIn("newly_added", violations[0])
        finally:
            formatting.FUNCTION_SIZE_EXEMPTIONS = old_exemptions

    def test_scan_mixed_responsibilities_flags_large_multi_mod_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _crate_root, source_dir = self.write_workspace_package(
                root, "pub mod orchestrator;\n", "crates/apps/example"
            )
            source = source_dir / "orchestrator.rs"
            lines = ["mod a;", "mod b;", "mod c;"]
            lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
            source.write_text("\n".join(lines) + "\n", encoding="utf-8")
            child_dir = source_dir / "orchestrator"
            child_dir.mkdir()
            for module in ("a", "b", "c"):
                (child_dir / f"{module}.rs").write_text(
                    "// child module\n", encoding="utf-8"
                )

            violations = formatting.scan_mixed_responsibilities()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])
            self.assertRegex(violations[0], r"\b3\b")
            self.assertIn("303", violations[0])

    def test_scan_mixed_responsibilities_skips_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lines = ["mod a;", "mod b;", "mod c;"]
            lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
            _crate_root, source_dir = self.write_workspace_package(
                root, "\n".join(lines) + "\n", "crates/apps/example"
            )
            for module in ("a", "b", "c"):
                (source_dir / f"{module}.rs").write_text(
                    "// child module\n", encoding="utf-8"
                )

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
                _crate_root, source_dir = self.write_workspace_package(
                    root, "pub mod orchestrator;\n", "crates/apps/example"
                )
                source = source_dir / "orchestrator.rs"
                lines = ["mod a;", "mod b;", "mod c;"]
                lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
                source.write_text("\n".join(lines) + "\n", encoding="utf-8")
                child_dir = source_dir / "orchestrator"
                child_dir.mkdir()
                for module in ("a", "b", "c"):
                    (child_dir / f"{module}.rs").write_text(
                        "// child module\n", encoding="utf-8"
                    )

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