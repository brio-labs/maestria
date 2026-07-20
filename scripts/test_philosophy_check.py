from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("philosophy-check.py")
SPEC = importlib.util.spec_from_file_location("philosophy_check", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load philosophy-check.py")
PHILOSOPHY_CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PHILOSOPHY_CHECK)


class PhilosophyCheckTests(unittest.TestCase):
    def setUp(self) -> None:
        self._old_globals = {
            "ROOT": PHILOSOPHY_CHECK.ROOT,
            "THIS_SCRIPT": PHILOSOPHY_CHECK.THIS_SCRIPT,
            "DOMAIN_ROOT": PHILOSOPHY_CHECK.DOMAIN_ROOT,
            "DOMAIN_SRC": PHILOSOPHY_CHECK.DOMAIN_SRC,
            "DOMAIN_MANIFEST": PHILOSOPHY_CHECK.DOMAIN_MANIFEST,
            "KERNEL_ROOTS": PHILOSOPHY_CHECK.KERNEL_ROOTS,
            "RESPONSIBILITY_MAPS": PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS,
        }

    def tearDown(self) -> None:
        for name, value in self._old_globals.items():
            setattr(PHILOSOPHY_CHECK, name, value)

    def configure_root(self, root: Path) -> None:
        kernel_root = root / "crates" / "kernel"
        domain_root = kernel_root / "maestria-domain"
        setattr(PHILOSOPHY_CHECK, "ROOT", root)
        setattr(PHILOSOPHY_CHECK, "THIS_SCRIPT", root / "scripts" / "philosophy-check.py")
        setattr(PHILOSOPHY_CHECK, "DOMAIN_ROOT", domain_root)
        setattr(PHILOSOPHY_CHECK, "DOMAIN_SRC", domain_root / "src")
        setattr(PHILOSOPHY_CHECK, "DOMAIN_MANIFEST", domain_root / "Cargo.toml")
        setattr(
            PHILOSOPHY_CHECK,
            "KERNEL_ROOTS",
            tuple(kernel_root / name for name in ("maestria-domain", "maestria-governance", "maestria-ports")),
        )
        setattr(
            PHILOSOPHY_CHECK,
            "RESPONSIBILITY_MAPS",
            {
                "crates/kernel/maestria-ports/src/traits.rs": (
                    "errors", "repositories", "lifecycle", "indexing",
                    "embedding", "harness", "graph", "web", "approval", "search",
                ),
            },
        )

    def test_scan_markers_reports_task_marker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("// " + "TO" + "DO" + ": remove marker\n", encoding="utf-8")

            self.assertEqual(PHILOSOPHY_CHECK.scan_markers(), ["crates/kernel/maestria-domain/src/lib.rs"])

    def test_scan_rust_lint_bypasses_reports_allow_attribute(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("#[allow(dead_code)]\nfn example() {}\n", encoding="utf-8")

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_rust_lint_bypasses(),
                ["crates/apps/example/src/lib.rs"],
            )

    def test_scan_rust_lint_bypasses_reports_cfg_attr_allow(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                '#[cfg_attr(test, allow(dead_code))]\nfn example() {}\n',
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_rust_lint_bypasses(),
                ["crates/apps/example/src/lib.rs"],
            )

    def test_scan_rust_forbidden_methods_reports_option_fallback(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "fn example(value: Option<u8>) { let _ = value.unwrap_or_default(); }\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_rust_forbidden_methods(),
                ["crates/apps/example/src/lib.rs contains a forbidden Option/Result failure method"],
            )

    def test_domain_scan_reports_runtime_tokens_and_production_failures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            domain = root / "crates" / "kernel" / "maestria-domain"
            source = domain / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            (domain / "Cargo.toml").write_text(
                "[package]\nname = \"maestria-domain\"\n[dependencies]\ntokio = \"1\"\n",
                encoding="utf-8",
            )
            source.write_text(
                "use std::fs;\n"
                "pub fn production_failure() { panic!(\"forbidden\"); }\n"
                "#[cfg(test)]\n"
                "mod tests { fn test_only() { value.unwrap(); } }\n",
                encoding="utf-8",
            )

            manifest_violations = PHILOSOPHY_CHECK.scan_domain_manifest()
            source_violations = PHILOSOPHY_CHECK.scan_domain_sources()

            self.assertEqual(
                manifest_violations,
                ["crates/kernel/maestria-domain/Cargo.toml contains forbidden dependency token tokio"],
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden domain token std::fs",
                source_violations,
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden failure token panic!(",
                source_violations,
            )
            self.assertNotIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden failure token unwrap(",
                source_violations,
            )

    def test_kernel_scan_covers_all_kernel_crates_and_failure_macros(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for name in ("maestria-domain", "maestria-governance", "maestria-ports"):
                crate = root / "crates" / "kernel" / name
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text("[package]\nname = \"test\"\n", encoding="utf-8")
            governance = root / "crates" / "kernel" / "maestria-governance"
            (governance / "Cargo.toml").write_text(
                "[package]\nname = \"test\"\n[dependencies]\nreqwest = \"1\"\n",
                encoding="utf-8",
            )
            (governance / "src" / "lib.rs").write_text(
                "pub fn invalid() { unreachable!(); }\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_kernel_manifests(),
                [
                    "crates/kernel/maestria-governance/Cargo.toml "
                    "contains forbidden dependency token reqwest"
                ],
            )
            self.assertEqual(
                PHILOSOPHY_CHECK.scan_kernel_sources(),
                [
                    "crates/kernel/maestria-governance/src/lib.rs "
                    "contains forbidden failure token unreachable!("
                ],
            )

    def test_responsibility_map_accepts_valid_trait_split(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            traits_dir = root / "crates" / "kernel" / "maestria-ports" / "src"
            traits_dir.mkdir(parents=True, exist_ok=True)
            traits_file = traits_dir / "traits.rs"
            modules = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS[
                "crates/kernel/maestria-ports/src/traits.rs"
            ]

            traits_lines = ["//! Responsibility map:"]
            traits_lines.extend(f"//! - `{module}`: test ownership." for module in modules)
            traits_lines.extend(f"mod {module};" for module in modules)
            traits_file.write_text("\n".join(traits_lines), encoding="utf-8")
            for module in modules:
                (traits_dir / f"{module}.rs").write_text("// test\n", encoding="utf-8")

            self.assertEqual(PHILOSOPHY_CHECK.scan_responsibility_maps(), [])

    def test_responsibility_map_reports_missing_module_declaration(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            traits_dir = root / "crates" / "kernel" / "maestria-ports" / "src"
            traits_dir.mkdir(parents=True, exist_ok=True)
            traits_file = traits_dir / "traits.rs"
            modules = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS[
                "crates/kernel/maestria-ports/src/traits.rs"
            ]

            traits_lines = ["//! Responsibility map:"]
            traits_lines.extend(f"//! - `{module}`: test ownership." for module in modules)
            traits_lines.extend(f"mod {module};" for module in modules if module != "repositories")
            traits_file.write_text("\n".join(traits_lines), encoding="utf-8")
            for module in modules:
                (traits_dir / f"{module}.rs").write_text("// test\n", encoding="utf-8")

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_responsibility_maps(),
                [
                    "crates/kernel/maestria-ports/src/traits.rs does not declare module 'repositories'"
                ],
            )


    def test_documentation_contract_requires_canonical_markers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for relative_path, markers in PHILOSOPHY_CHECK.CANONICAL_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                sections = PHILOSOPHY_CHECK.CANONICAL_DOC_SECTIONS[relative_path]
                path.write_text("\n".join((*markers, *sections)), encoding="utf-8")
            for relative_path, markers in PHILOSOPHY_CHECK.POLICY_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("\n".join(markers), encoding="utf-8")

            self.assertEqual(PHILOSOPHY_CHECK.scan_documentation_contract(), [])

            missing = root / "docs" / "SEARCH.md"
            missing.write_text("SearchPlan only", encoding="utf-8")
            violations = PHILOSOPHY_CHECK.scan_documentation_contract()
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
            policy_violations = PHILOSOPHY_CHECK.scan_documentation_contract()
            self.assertIn(
                "docs/PHILOSOPHY.md is missing required marker '42. Search traces'",
                policy_violations,
            )

    def test_documentation_contract_rejects_external_truth_wording(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for relative_path, markers in PHILOSOPHY_CHECK.CANONICAL_DOC_MARKERS.items():
                path = root / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                sections = PHILOSOPHY_CHECK.CANONICAL_DOC_SECTIONS[relative_path]
                path.write_text("\n".join((*markers, *sections)), encoding="utf-8")
            for relative_path, markers in PHILOSOPHY_CHECK.POLICY_DOC_MARKERS.items():
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
                PHILOSOPHY_CHECK.scan_documentation_contract(),
            )

            self.assertIn(
                "docs/ARCHITECTURE.md contains prohibited external-truth wording 'domain owns truth'",
                PHILOSOPHY_CHECK.scan_documentation_contract(),
            )

    def test_exemption_expiry_is_enforced_at_target_version(self) -> None:
        old_module = PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS
        old_adr = PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = {
                "crates/example/src/large.rs": "v0.7.0",
            }
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = {}
            self.assertEqual(PHILOSOPHY_CHECK.scan_exemption_expiry("0.6.1"), [])
            self.assertEqual(len(PHILOSOPHY_CHECK.scan_exemption_expiry("0.7.0")), 1)
            self.assertEqual(len(PHILOSOPHY_CHECK.scan_exemption_expiry("0.8.0")), 1)
        finally:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = old_module
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = old_adr

    def test_exemption_expiry_rejects_malformed_target(self) -> None:
        old_module = PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS
        old_adr = PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = {
                "crates/example/src/large.rs": "v0.7",
            }
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = {}
            violations = PHILOSOPHY_CHECK.scan_exemption_expiry("0.6.1")
            self.assertEqual(len(violations), 1)
            self.assertIn("malformed", violations[0])
        finally:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = old_module
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = old_adr

    def test_workspace_version_reads_workspace_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                "[workspace.package]\nversion = \"0.6.1\"\n\n[workspace]\nmembers = []\n",
                encoding="utf-8",
            )
            self.assertEqual(PHILOSOPHY_CHECK.workspace_version(), "0.6.1")

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

            violations = PHILOSOPHY_CHECK.scan_module_sizes()

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
            source.write_text("\n".join("fn test_case() {}" for _ in range(901)), encoding="utf-8")

            violations = PHILOSOPHY_CHECK.scan_module_sizes()

            self.assertEqual(
                violations,
                [
                    "crates/core/maestria-core/tests/large.rs has "
                    "901 physical lines (limit 900)"
                ],
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
            old_maps = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS
            PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = {
                "crates/kernel/maestria-domain/src/lib.rs": ("foo",),
            }
            try:
                violations = PHILOSOPHY_CHECK.scan_facade_boundaries()
                self.assertEqual(len(violations), 1)
                self.assertIn("implementation body", violations[0])
            finally:
                PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = old_maps

    def test_facade_boundary_accepts_pure_facade(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "kernel" / "maestria-domain" / "src"
            lib_dir.mkdir(parents=True)
            lib_rs = lib_dir / "lib.rs"
            lib_rs.write_text(
                "pub mod foo;\npub mod bar;\npub use foo::*;\npub use bar::*;\n",
                encoding="utf-8",
            )
            (lib_dir / "foo.rs").write_text("// foo\n", encoding="utf-8")
            (lib_dir / "bar.rs").write_text("// bar\n", encoding="utf-8")
            old_maps = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS
            PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = {
                "crates/kernel/maestria-domain/src/lib.rs": ("foo", "bar"),
            }
            try:
                violations = PHILOSOPHY_CHECK.scan_facade_boundaries()
                self.assertEqual(violations, [])
            finally:
                PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = old_maps

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
            old_maps = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS
            PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = {
                "crates/kernel/maestria-domain/src/lib.rs": ("foo",),
            }
            try:
                violations = PHILOSOPHY_CHECK.scan_cohesion()
                self.assertEqual(len(violations), 1)
                self.assertIn("cohesion signal", violations[0])
            finally:
                PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = old_maps

    def test_production_strip_line_comments_keeps_doc_comments(self) -> None:
        body = "//! doc comment\n// normal comment\npub fn foo() {}\n"
        result = PHILOSOPHY_CHECK.production_strip_line_comments(body)
        self.assertIn("//! doc comment", result)
        self.assertNotIn("// normal comment", result)
        self.assertIn("pub fn foo", result)


if __name__ == "__main__":
    unittest.main()
