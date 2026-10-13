"""Tests for the philosophy_check contract tests family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import contract_tests, dependency_graph, formatting
from philosophy_check_testbase import PhilosophyCheckFixture


class DocumentationContractTests(PhilosophyCheckFixture):
    def write_workspace_facade(self, root: Path, contents: str) -> tuple[Path, Path]:
        (root / "Cargo.toml").write_text(
            '[workspace]\nmembers = ["external/example"]\n', encoding="utf-8"
        )
        crate = root / "external" / "example"
        source_dir = crate / "src"
        source_dir.mkdir(parents=True)
        (crate / "Cargo.toml").write_text(
            '[package]\nname = "example"\nversion = "0.1.0"\nedition = "2021"\n',
            encoding="utf-8",
        )
        source = source_dir / "lib.rs"
        source.write_text(contents, encoding="utf-8")
        return source, source_dir

    def write_cfg_platform_facade(
        self, root: Path, *, include_windows_source: bool
    ) -> tuple[Path, Path]:
        source, source_dir = self.write_workspace_facade(
            root,
            "//! Responsibility map:\n"
            "//! - `platform`: selects the host-specific backend.\n"
            "#[cfg(unix)]\n"
            '#[path = "unix.rs"]\n'
            "mod platform;\n"
            "#[cfg(windows)]\n"
            '#[path = "windows.rs"]\n'
            "mod platform;\n",
        )
        (source_dir / "unix.rs").write_text(
            "pub fn backend() {}\n", encoding="utf-8"
        )
        if include_windows_source:
            (source_dir / "windows.rs").write_text(
                "pub fn backend() {}\n", encoding="utf-8"
            )
        return source, source_dir

    def test_responsibility_map_accepts_new_documented_module_without_inventory(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `existing`: constructs stable search requests.\n"
                "//! - `new_feature`: owns a newly added parsing boundary.\n"
                "pub mod existing;\npub mod new_feature;\n",
            )
            for module in ("existing", "new_feature"):
                (source_dir / f"{module}.rs").write_text(
                    "// production module\n", encoding="utf-8"
                )

            self.assertEqual(contract_tests.scan_responsibility_maps(), [])

    def test_responsibility_map_reports_undeclared_documented_module(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `present`: owns the declared service boundary.\n"
                "//! - `extra`: handles an undeclared side effect.\n"
                "pub mod present;\n",
            )
            (source_dir / "present.rs").write_text("// present\n", encoding="utf-8")
            (source_dir / "extra.rs").write_text("// extra\n", encoding="utf-8")

            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:3", violations[0]
            )
            self.assertIn("extra", violations[0])

    def test_responsibility_map_reports_production_module_without_role(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `present`: owns the declared service boundary.\n"
                "pub mod present;\npub mod undocumented;\n",
            )
            for module in ("present", "undocumented"):
                (source_dir / f"{module}.rs").write_text(
                    "// production module\n", encoding="utf-8"
                )

            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:4", violations[0]
            )
            self.assertIn("undocumented", violations[0])

    def test_responsibility_map_rejects_duplicate_entries_with_lines(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `search`: builds bounded search requests.\n"
                "//! - `search`: formats search evidence.\n"
                "pub mod search;\n",
            )
            (source_dir / "search.rs").write_text("// search\n", encoding="utf-8")

            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:3", violations[0]
            )
            self.assertIn("search", violations[0])

    def test_responsibility_map_rejects_empty_role_with_source_line(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `search`:\n"
                "pub mod search;\n",
            )
            (source_dir / "search.rs").write_text("// search\n", encoding="utf-8")

            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:2", violations[0]
            )
            self.assertIn("search", violations[0])

    def test_documentation_contract_requires_canonical_markers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for (
                relative_path,
                markers,
            ) in contract_tests.CANONICAL_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                sections = contract_tests.CANONICAL_DOC_SECTIONS[relative_path]
                path.write_text("\n".join((*markers, *sections)), encoding="utf-8")
            for relative_path, markers in contract_tests.POLICY_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("\n".join(markers), encoding="utf-8")

            self.assertEqual(contract_tests.scan_documentation_contract(), [])

            missing = root / "docs" / "SEARCH.md"
            missing.write_text("SearchPlan only", encoding="utf-8")
            violations = contract_tests.scan_documentation_contract()
            self.assertIn(
                "docs/SEARCH.md is missing required marker 'SearchTraceId'",
                violations,
            )
            self.assertIn(
                "docs/SEARCH.md is missing required section '## Search Boundary Objects'",
                violations,
            )
            policy = root / "docs" / "PHILOSOPHY.md"
            policy.write_text("41. Search plans", encoding="utf-8")
            policy_violations = contract_tests.scan_documentation_contract()
            self.assertIn(
                "docs/PHILOSOPHY.md is missing required marker '42. Search traces'",
                policy_violations,
            )

    def test_documentation_contract_rejects_external_truth_wording(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for (
                relative_path,
                markers,
            ) in contract_tests.CANONICAL_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                sections = contract_tests.CANONICAL_DOC_SECTIONS[relative_path]
                path.write_text("\n".join((*markers, *sections)), encoding="utf-8")
            for relative_path, markers in contract_tests.POLICY_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("\n".join(markers), encoding="utf-8")

            architecture = root / "docs" / "ARCHITECTURE.md"
            architecture.write_text(
                "authoritative state; external factual truth; domain owns truth",
                encoding="utf-8",
            )

            legacy = root / "docs" / "architecture" / "book-iv-ecosystem.md"
            legacy.parent.mkdir(parents=True, exist_ok=True)
            legacy.write_text("This projection is a truth owner.", encoding="utf-8")
            self.assertIn(
                "docs/architecture/book-iv-ecosystem.md contains prohibited external-truth wording 'truth owner'",
                contract_tests.scan_documentation_contract(),
            )

            self.assertIn(
                "docs/ARCHITECTURE.md contains prohibited external-truth wording 'domain owns truth'",
                contract_tests.scan_documentation_contract(),
            )

    def test_facade_boundary_reports_impl_in_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "kernel" / "sillage-domain" / "src"
            lib_dir.mkdir(parents=True)
            lib_rs = lib_dir / "lib.rs"
            lib_rs.write_text(
                "pub mod foo;\npub fn helper() -> i32 { 42 }\n",
                encoding="utf-8",
            )
            violations = dependency_graph.scan_facade_boundaries()
            self.assertEqual(len(violations), 1)
            self.assertIn(lib_rs.relative_to(root).as_posix(), violations[0])

    def test_facade_boundary_accepts_pure_facade(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            _source, lib_dir = self.write_workspace_facade(
                root,
                "pub mod foo;\npub mod bar;\n"
                "pub use foo::Foo;\npub use bar::Bar;\n",
            )
            (lib_dir / "foo.rs").write_text("// foo\n", encoding="utf-8")
            (lib_dir / "bar.rs").write_text("// bar\n", encoding="utf-8")

            violations = dependency_graph.scan_facade_boundaries()
            self.assertEqual(violations, [])

    def test_responsibility_maps_ignore_test_only_and_check_following_production_modules(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `split`: separates route parsing from query ownership.\n"
                "#[cfg(test)]\n"
                "mod tests {\n"
                "    mod fixture;\n"
                "}\n"
                "#[cfg(any(test, feature = \"shipping\"))]\n"
                "mod shipping;\n"
                "pub\nmod split;\n",
            )
            (source_dir / "split.rs").write_text("// split\n", encoding="utf-8")
            (source_dir / "shipping.rs").write_text("// shipping\n", encoding="utf-8")

            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:8", violations[0]
            )
            self.assertIn("shipping", violations[0])

    def test_responsibility_map_accepts_cfg_alternative_module_sources(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            self.write_cfg_platform_facade(root, include_windows_source=True)

            self.assertEqual(contract_tests.scan_responsibility_maps(), [])

    def test_responsibility_map_checks_each_cfg_alternative_source(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, _source_dir = self.write_cfg_platform_facade(
                root, include_windows_source=False
            )

            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(
                f"{source.relative_to(root).as_posix()}:8", violations[0]
            )
            self.assertIn("windows.rs", violations[0])

    def test_responsibility_map_discovery_ignores_ordinary_rust_doc_bullets(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            facade, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `repository_index`: exposes repository-index browsing.\n"
                "pub mod repository_index;\n",
            )
            module_source = source_dir / "repository_index.rs"
            module_source.write_text(
                "//! Command-line options:\n"
                "//! - `--all`: include every indexed entry.\n"
                "//! - `--yes`: accept confirmation prompts.\n"
                "pub fn options() {}\n",
                encoding="utf-8",
            )

            map_sources = contract_tests.responsibility_map_source_paths()
            self.assertIn(facade, map_sources)
            self.assertNotIn(module_source, map_sources)
            self.assertEqual(contract_tests.scan_responsibility_maps(), [])

    def test_cargo_facade_without_map_header_still_requires_a_map(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root, "mod implementation;\n"
            )
            (source_dir / "implementation.rs").write_text(
                "// production implementation\n", encoding="utf-8"
            )

            self.assertIn(
                source, contract_tests.responsibility_map_source_paths()
            )
            violations = contract_tests.scan_responsibility_maps()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])

    def test_cohesion_counts_production_after_test_module_without_waiving_threshold(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source, source_dir = self.write_workspace_facade(
                root,
                "//! Responsibility map:\n"
                "//! - `foo`: exposes the bounded search request interface.\n"
                "#[cfg(test)]\n"
                "mod tests {\n"
                "    fn fixture() {}\n"
                "}\n"
                "pub mod foo;\n"
                + "pub use foo::*;\n" * 17,
            )
            (source_dir / "foo.rs").write_text("// foo\n", encoding="utf-8")

            violations = formatting.scan_cohesion()
            self.assertEqual(len(violations), 1)
            self.assertIn(source.relative_to(root).as_posix(), violations[0])