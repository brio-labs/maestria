"""Tests for the philosophy_check contract tests family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import contract_tests, dependency_graph, formatting, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class DocumentationContractTests(PhilosophyCheckFixture):

    def test_responsibility_map_accepts_valid_trait_split(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            traits_dir = root / "crates" / "kernel" / "maestria-ports" / "src"
            traits_dir.mkdir(parents=True, exist_ok=True)
            traits_file = traits_dir / "traits.rs"
            modules = contract_tests.RESPONSIBILITY_MAPS[
                "crates/kernel/maestria-ports/src/traits.rs"
            ]

            traits_lines = ["//! Responsibility map:"]
            traits_lines.extend(
                f"//! - `{module}`: test ownership." for module in modules
            )
            traits_lines.extend(f"mod {module};" for module in modules)
            traits_file.write_text("\n".join(traits_lines), encoding="utf-8")
            for module in modules:
                (traits_dir / f"{module}.rs").write_text("// test\n", encoding="utf-8")

            self.assertEqual(contract_tests.scan_responsibility_maps(), [])

    def test_responsibility_map_reports_missing_module_declaration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            traits_dir = root / "crates" / "kernel" / "maestria-ports" / "src"
            traits_dir.mkdir(parents=True, exist_ok=True)
            traits_file = traits_dir / "traits.rs"
            modules = contract_tests.RESPONSIBILITY_MAPS[
                "crates/kernel/maestria-ports/src/traits.rs"
            ]

            traits_lines = ["//! Responsibility map:"]
            traits_lines.extend(
                f"//! - `{module}`: test ownership." for module in modules
            )
            traits_lines.extend(
                f"mod {module};" for module in modules if module != "repositories"
            )
            traits_file.write_text("\n".join(traits_lines), encoding="utf-8")
            for module in modules:
                (traits_dir / f"{module}.rs").write_text("// test\n", encoding="utf-8")

            self.assertEqual(
                contract_tests.scan_responsibility_maps(),
                [
                    "crates/kernel/maestria-ports/src/traits.rs does not declare module 'repositories'"
                ],
            )

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
            lib_dir = root / "crates" / "kernel" / "maestria-domain" / "src"
            lib_dir.mkdir(parents=True)
            lib_rs = lib_dir / "lib.rs"
            lib_rs.write_text(
                "pub mod foo;\npub fn helper() -> i32 { 42 }\n",
                encoding="utf-8",
            )
            old_maps = contract_tests.RESPONSIBILITY_MAPS
            contract_tests.RESPONSIBILITY_MAPS = {
                "crates/kernel/maestria-domain/src/lib.rs": ("foo",),
            }
            try:
                violations = dependency_graph.scan_facade_boundaries()
                self.assertEqual(len(violations), 1)
                self.assertIn("implementation body", violations[0])
            finally:
                contract_tests.RESPONSIBILITY_MAPS = old_maps

    def test_facade_boundary_accepts_pure_facade(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "kernel" / "maestria-domain" / "src"
            lib_dir.mkdir(parents=True)
            lib_rs = lib_dir / "lib.rs"
            lib_rs.write_text(
                "pub mod foo;\npub mod bar;\npub use foo::Foo;\npub use bar::Bar;\n",
                encoding="utf-8",
            )
            (lib_dir / "foo.rs").write_text("// foo\n", encoding="utf-8")
            (lib_dir / "bar.rs").write_text("// bar\n", encoding="utf-8")
            old_maps = contract_tests.RESPONSIBILITY_MAPS
            contract_tests.RESPONSIBILITY_MAPS = {
                "crates/kernel/maestria-domain/src/lib.rs": ("foo", "bar"),
            }
            try:
                violations = dependency_graph.scan_facade_boundaries()
                self.assertEqual(violations, [])
            finally:
                contract_tests.RESPONSIBILITY_MAPS = old_maps

    def test_responsibility_maps_detect_split_pub_mod_and_non_test_cfg(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "//! Responsibility map:\n"
                "//! - `split`: split declaration.\n"
                "#[cfg(test)]\nmod tests;\n"
                "#[cfg(any(test, feature = \"shipping\"))]\nmod shipping;\n"
                "pub\nmod split;\n",
                encoding="utf-8",
            )
            (source.parent / "split.rs").write_text("", encoding="utf-8")
            old_maps = contract_tests.RESPONSIBILITY_MAPS
            contract_tests.RESPONSIBILITY_MAPS = {
                "crates/kernel/example/src/lib.rs": ("split",)
            }
            try:
                violations = contract_tests.scan_responsibility_maps()
                self.assertIn(
                    "crates/kernel/example/src/lib.rs responsibility map omits module 'shipping'",
                    violations,
                )
                self.assertNotIn(
                    "crates/kernel/example/src/lib.rs responsibility map omits module 'tests'",
                    violations,
                )
            finally:
                contract_tests.RESPONSIBILITY_MAPS = old_maps

    def test_responsibility_maps_reject_production_module_omitted_from_configuration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["external/example"]\n',
                encoding="utf-8",
            )
            crate = root / "external" / "example"
            (crate / "src").mkdir(parents=True)
            (crate / "Cargo.toml").write_text(
                '[package]\nname = "example"\n', encoding="utf-8"
            )
            (crate / "src" / "lib.rs").write_text("mod implementation;\n", encoding="utf-8")
            old_maps = contract_tests.RESPONSIBILITY_MAPS
            contract_tests.RESPONSIBILITY_MAPS = {}
            try:
                self.assertEqual(
                    contract_tests.scan_responsibility_maps(),
                    ["external/example/src/lib.rs production module has no configured responsibility map"],
                )
            finally:
                contract_tests.RESPONSIBILITY_MAPS = old_maps

    def test_cohesion_reports_dense_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "kernel" / "maestria-domain" / "src"
            lib_dir.mkdir(parents=True)
            lib_rs = lib_dir / "lib.rs"
            # 17 meaningful lines with only 1 module = high density
            lib_rs.write_text(
                "pub use foo::*;\n" * 17,
                encoding="utf-8",
            )
            (lib_dir / "foo.rs").write_text("// foo\n", encoding="utf-8")
            old_maps = contract_tests.RESPONSIBILITY_MAPS
            contract_tests.RESPONSIBILITY_MAPS = {
                "crates/kernel/maestria-domain/src/lib.rs": ("foo",),
            }
            try:
                violations = formatting.scan_cohesion()
                self.assertEqual(len(violations), 1)
                self.assertIn("cohesion signal", violations[0])
            finally:
                contract_tests.RESPONSIBILITY_MAPS = old_maps
