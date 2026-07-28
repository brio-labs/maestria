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
        setattr(
            PHILOSOPHY_CHECK, "THIS_SCRIPT", root / "scripts" / "philosophy-check.py"
        )
        setattr(PHILOSOPHY_CHECK, "DOMAIN_ROOT", domain_root)
        setattr(PHILOSOPHY_CHECK, "DOMAIN_SRC", domain_root / "src")
        setattr(PHILOSOPHY_CHECK, "DOMAIN_MANIFEST", domain_root / "Cargo.toml")
        setattr(
            PHILOSOPHY_CHECK,
            "KERNEL_ROOTS",
            tuple(
                kernel_root / name
                for name in ("maestria-domain", "maestria-governance", "maestria-ports")
            ),
        )
        setattr(
            PHILOSOPHY_CHECK,
            "RESPONSIBILITY_MAPS",
            {
                "crates/kernel/maestria-ports/src/traits.rs": (
                    "errors",
                    "repositories",
                    "lifecycle",
                    "indexing",
                    "embedding",
                    "harness",
                    "graph",
                    "web",
                    "approval",
                    "search",
                ),
            },
        )

    def test_scan_markers_reports_task_marker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "// " + "TO" + "DO" + ": remove marker\n", encoding="utf-8"
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_markers(),
                ["crates/kernel/maestria-domain/src/lib.rs"],
            )

    def test_scan_rust_lint_bypasses_reports_allow_attribute(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "#[allow(dead_code)]\nfn example() {}\n", encoding="utf-8"
            )

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
                "#[cfg_attr(test, allow(dead_code))]\nfn example() {}\n",
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
            test_source = source.parent / "tests.rs"
            test_source.write_text(
                "mod tests { fn test_only(value: Option<u8>) { let _ = value.unwrap(); } }\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_rust_forbidden_methods(),
                [
                    "crates/apps/example/src/lib.rs contains a forbidden Option/Result failure method",
                    "crates/apps/example/src/tests.rs contains a forbidden Option/Result failure method",
                ],
            )

    def test_scan_unbounded_channels_reports_constructor_and_types(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "use tokio::sync::mpsc;\n"
                "fn example() -> mpsc::UnboundedSender<u8> {\n"
                "    let (sender, _receiver) = mpsc::unbounded_channel();\n"
                "    sender\n"
                "}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_unbounded_channels(),
                [
                    "crates/apps/example/src/lib.rs contains an unbounded internal channel"
                ],
            )

    def test_scan_unbounded_channels_covers_std_and_crossbeam(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "channels.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "fn example() {\n"
                "    let _ = std::sync::mpsc::channel::<u8>();\n"
                "    let _ = crossbeam_channel::unbounded::<u8>();\n"
                "}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_unbounded_channels(),
                [
                    "crates/apps/example/src/channels.rs contains an unbounded internal channel"
                ],
            )

    def test_domain_scan_reports_runtime_tokens_and_production_failures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            domain = root / "crates" / "kernel" / "maestria-domain"
            source = domain / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            (domain / "Cargo.toml").write_text(
                '[package]\nname = "maestria-domain"\n[dependencies]\ntokio = "1"\n',
                encoding="utf-8",
            )
            source.write_text(
                "use std::fs;\n"
                'pub fn production_failure() { panic!("forbidden"); }\n'
                "#[cfg(test)]\n"
                "mod tests { fn test_only() { value.unwrap(); } }\n",
                encoding="utf-8",
            )

            manifest_violations = PHILOSOPHY_CHECK.scan_domain_manifest()
            source_violations = PHILOSOPHY_CHECK.scan_domain_sources()

            self.assertEqual(
                manifest_violations,
                [
                    "crates/kernel/maestria-domain/Cargo.toml contains forbidden dependency token tokio"
                ],
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden domain token std::fs",
                source_violations,
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden failure token panic!(",
                source_violations,
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden failure token unwrap(",
                source_violations,
            )

    def test_manifest_dependencies_normalizes_all_dependency_tables(self) -> None:
        content = """
[dependencies]
renamed_sha = { package = "sha2", version = "1" }
[dev-dependencies]
tokio_alias = { package = "tokio", version = "1" }
[build-dependencies]
build_tool = "1"
[target.'cfg(unix)'.dependencies]
target_alias = { package = "Reqwest", version = "1" }
[target.'cfg(unix)'.dev-dependencies]
dev_alias = { package = "unknown-package", version = "1" }
"""
        self.assertEqual(
            PHILOSOPHY_CHECK._manifest_dependencies(content),
            {"sha2", "tokio", "build-tool", "reqwest", "unknown-package"},
        )

    def test_kernel_manifest_rejects_unknown_and_forbidden_target_build_dev_dependencies(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for name in ("maestria-domain", "maestria-governance", "maestria-ports"):
                crate = root / "crates" / "kernel" / name
                (crate / "src").mkdir(parents=True)
                dependency = {
                    "maestria-domain": 'sha2 = "0.10"',
                    "maestria-governance": 'maestria_domain = { package = "maestria-domain", path = "../../domain" }',
                    "maestria-ports": 'maestria_domain = { package = "maestria-domain", path = "../../domain" }',
                }[name]
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "test"\n[dependencies]\n{dependency}\n',
                    encoding="utf-8",
                )
            manifest = root / "crates" / "kernel" / "maestria-domain" / "Cargo.toml"
            manifest.write_text(
                '[package]\nname = "test"\n[dependencies]\nsha2 = "0.10"\n'
                '[target."cfg(unix)".dependencies]\nrenamed = { package = "unknown-ext", version = "1" }\n'
                '[build-dependencies]\nbuilder = "1"\n'
                '[dev-dependencies]\ntokio_alias = { package = "tokio", version = "1" }\n',
                encoding="utf-8",
            )

            violations = PHILOSOPHY_CHECK.scan_kernel_manifests()

            self.assertIn(
                "crates/kernel/maestria-domain/Cargo.toml contains disallowed kernel dependency unknown-ext",
                violations,
            )
            self.assertIn(
                "crates/kernel/maestria-domain/Cargo.toml contains disallowed kernel dependency builder",
                violations,
            )
            self.assertIn(
                "crates/kernel/maestria-domain/Cargo.toml contains forbidden dependency token tokio",
                violations,
            )

    def test_kernel_manifest_allows_only_declared_kernel_dependencies(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            manifests = {
                "maestria-domain": '[dependencies]\nsha2 = "0.10"\n',
                "maestria-governance": '[dependencies]\nmaestria_domain = { package = "maestria-domain", path = "../../domain" }\n',
                "maestria-ports": '[dependencies]\nmaestria_domain = { package = "maestria-domain", path = "../../domain" }\n',
            }
            for name, dependencies in manifests.items():
                crate = root / "crates" / "kernel" / name
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "test"\n{dependencies}',
                    encoding="utf-8",
                )
            self.assertEqual(PHILOSOPHY_CHECK.scan_kernel_manifests(), [])

    def test_kernel_scan_covers_all_kernel_crates_and_failure_macros(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            for name in ("maestria-domain", "maestria-governance", "maestria-ports"):
                crate = root / "crates" / "kernel" / name
                (crate / "src").mkdir(parents=True)
                dependency = {
                    "maestria-domain": 'sha2 = "0.10"',
                    "maestria-governance": 'maestria_domain = { package = "maestria-domain", path = "../../domain" }',
                    "maestria-ports": 'maestria_domain = { package = "maestria-domain", path = "../../domain" }',
                }[name]
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "test"\n[dependencies]\n{dependency}\n',
                    encoding="utf-8",
                )
            governance = root / "crates" / "kernel" / "maestria-governance"
            (governance / "Cargo.toml").write_text(
                '[package]\nname = "test"\n[dependencies]\n'
                'maestria_domain = { package = "maestria-domain", path = "../../domain" }\n'
                'reqwest = "1"\n',
                encoding="utf-8",
            )
            (governance / "src" / "lib.rs").write_text(
                "pub fn invalid() { unreachable!(); }\n",
                encoding="utf-8",
            )
            (governance / "src" / "tests.rs").write_text(
                "mod tests { fn test_only() { unreachable!(); } }\n",
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
                    "contains forbidden failure token unreachable!(",
                    "crates/kernel/maestria-governance/src/tests.rs "
                    "contains forbidden failure token unreachable!(",
                ],
            )

    def test_kernel_scan_rejects_std_network_and_unsafe_rust(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            domain = root / "crates" / "kernel" / "maestria-domain"
            source = domain / "src" / "network.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "use std::net::TcpStream;\n"
                "// unsafe fn in a comment is not a violation by itself\n"
                "pub unsafe fn connect() {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_kernel_sources(),
                [
                    "crates/kernel/maestria-domain/src/network.rs contains "
                    "forbidden kernel token std::net",
                    "crates/kernel/maestria-domain/src/network.rs contains "
                    "forbidden unsafe Rust",
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
            traits_lines.extend(
                f"//! - `{module}`: test ownership." for module in modules
            )
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
                PHILOSOPHY_CHECK.scan_responsibility_maps(),
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
            ) in PHILOSOPHY_CHECK.CANONICAL_DOC_MARKERS.items():
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
            for (
                relative_path,
                markers,
            ) in PHILOSOPHY_CHECK.CANONICAL_DOC_MARKERS.items():
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
        old_fn = PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS
        old_mixed = PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = {
                "crates/example/src/large.rs": "v0.7.0",
            }
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = {}
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = {}
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = {}
            self.assertEqual(PHILOSOPHY_CHECK.scan_exemption_expiry("0.6.1"), [])
            self.assertEqual(len(PHILOSOPHY_CHECK.scan_exemption_expiry("0.7.0")), 1)
            self.assertEqual(len(PHILOSOPHY_CHECK.scan_exemption_expiry("0.8.0")), 1)
        finally:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = old_module
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = old_adr
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = old_fn
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed

    def test_exemption_expiry_rejects_malformed_target(self) -> None:
        old_module = PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS
        old_adr = PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS
        old_fn = PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS
        old_mixed = PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = {
                "crates/example/src/large.rs": "v0.7",
            }
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = {}
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = {}
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = {}
            violations = PHILOSOPHY_CHECK.scan_exemption_expiry("0.6.1")
            self.assertEqual(len(violations), 1)
            self.assertIn("malformed", violations[0])
        finally:
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = old_module
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = old_adr
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = old_fn
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed

    def test_workspace_version_reads_workspace_package(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[workspace.package]\nversion = "0.6.1"\n\n[workspace]\nmembers = []\n',
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
            source.write_text(
                "\n".join("fn test_case() {}" for _ in range(901)), encoding="utf-8"
            )

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
                "pub mod foo;\npub mod bar;\npub use foo::Foo;\npub use bar::Bar;\n",
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

    def test_facade_boundary_discovers_unlisted_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "ecosystem" / "example" / "src"
            lib_dir.mkdir(parents=True)
            (lib_dir / "lib.rs").write_text(
                "pub fn unlisted_helper() -> i32 { 42 }\n", encoding="utf-8"
            )

            violations = PHILOSOPHY_CHECK.scan_facade_boundaries()

            self.assertEqual(len(violations), 1)
            self.assertIn("crates/ecosystem/example/src/lib.rs", violations[0])

    def test_facade_boundary_accepts_unlisted_pure_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "ecosystem" / "pure" / "src"
            lib_dir.mkdir(parents=True)
            (lib_dir / "lib.rs").write_text(
                "mod implementation;\npub use implementation::Helper;\n",
                encoding="utf-8",
            )
            (lib_dir / "implementation.rs").write_text(
                "pub fn helper() -> i32 { 42 }\n", encoding="utf-8"
            )

            self.assertEqual(PHILOSOPHY_CHECK.scan_facade_boundaries(), [])

    def test_facade_boundary_rejects_syntax_bypass_items(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "kernel" / "example" / "src"
            lib_dir.mkdir(parents=True)
            (lib_dir / "lib.rs").write_text(
                "pub fn generic<T>() {}\n"
                "pub const fn constant() {}\n"
                "pub trait Trait {}\n"
                "pub union Union { value: u8 }\n"
                "macro_rules! generated { () => {} }\n"
                "pub type Alias = u8;\n"
                "pub use implementation::{Helper, *};\n"
                "pub use implementation::*;\n",
                encoding="utf-8",
            )
            violations = PHILOSOPHY_CHECK.scan_facade_boundaries()
            self.assertEqual(len(violations), 1)
            self.assertIn("8 implementation item(s)", violations[0])

    def test_facade_boundary_rejects_whitespace_separated_wildcard(self) -> None:
        self.assertFalse(
            PHILOSOPHY_CHECK._facade_item_allowed("pub use x::\n\t *;")
        )

    def test_production_lib_paths_discovers_external_workspace_members_and_excludes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "root"\n\n[workspace]\n'
                'members = ["external/*"]\nexclude = ["external/excluded"]\n',
                encoding="utf-8",
            )
            (root / "src").mkdir()
            (root / "src" / "lib.rs").write_text("mod root_impl;\n", encoding="utf-8")
            for name in ("kept", "excluded"):
                crate = root / "external" / name
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\n', encoding="utf-8"
                )
                (crate / "src" / "lib.rs").write_text("mod implementation;\n", encoding="utf-8")
            discovered = {
                path.relative_to(root).as_posix()
                for path in PHILOSOPHY_CHECK.production_lib_paths()
            }
            self.assertEqual(
                discovered,
                {"src/lib.rs", "external/kept/src/lib.rs"},
            )

    def test_production_lib_paths_discovers_implicit_in_tree_path_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n', encoding="utf-8"
            )
            member = root / "member"
            implicit = root / "implicit"
            for crate, dependency in ((member, 'implicit = { path = "../implicit" }'), (implicit, "")):
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{crate.name}"\n[dependencies]\n{dependency}\n',
                    encoding="utf-8",
                )
                (crate / "src" / "lib.rs").write_text("mod implementation;\n", encoding="utf-8")
            discovered = {
                path.relative_to(root).as_posix()
                for path in PHILOSOPHY_CHECK.production_lib_paths()
            }
            self.assertEqual(discovered, {"member/src/lib.rs", "implicit/src/lib.rs"})

    def test_production_lib_paths_resolves_inherited_workspace_path_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                "[workspace]\n"
                'members = ["member"]\n\n'
                "[workspace.dependencies]\n"
                'implicit_alias = { package = "implicit", path = "implicit" }\n',
                encoding="utf-8",
            )
            member = root / "member"
            implicit = root / "implicit"
            (member / "src").mkdir(parents=True)
            (implicit / "src").mkdir(parents=True)
            (member / "Cargo.toml").write_text(
                '[package]\nname = "member"\n[dependencies]\n'
                'implicit_alias = { workspace = true }\n',
                encoding="utf-8",
            )
            (implicit / "Cargo.toml").write_text(
                '[package]\nname = "implicit"\n', encoding="utf-8"
            )
            (member / "src" / "lib.rs").write_text("mod implementation;\n", encoding="utf-8")
            (implicit / "src" / "lib.rs").write_text("mod implementation;\n", encoding="utf-8")
            discovered = {
                path.relative_to(root).as_posix()
                for path in PHILOSOPHY_CHECK.production_lib_paths()
            }
            self.assertEqual(discovered, {"member/src/lib.rs", "implicit/src/lib.rs"})

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
            old_maps = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS
            PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = {
                "crates/kernel/example/src/lib.rs": ("split",)
            }
            try:
                violations = PHILOSOPHY_CHECK.scan_responsibility_maps()
                self.assertIn(
                    "crates/kernel/example/src/lib.rs responsibility map omits module 'shipping'",
                    violations,
                )
                self.assertNotIn(
                    "crates/kernel/example/src/lib.rs responsibility map omits module 'tests'",
                    violations,
                )
            finally:
                PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = old_maps

    def test_module_scanner_handles_same_line_attributes_without_nested_matches(self) -> None:
        modules = PHILOSOPHY_CHECK._top_level_module_declarations(
            "#[cfg(test)] mod tests; "
            "#[cfg(any(test, feature = \"shipping\"))] mod shipping; "
            "pub\nmod split; "
            "mod outer { mod nested; }\n"
        )
        self.assertEqual(modules, {"shipping", "split"})

    def test_kernel_manifest_resolves_inherited_renamed_workspace_dependency(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                "[workspace]\n"
                'members = ["crates/kernel/*"]\n\n'
                "[workspace.dependencies]\n"
                'network_alias = { package = "reqwest", version = "1" }\n',
                encoding="utf-8",
            )
            for name, dependency in {
                "maestria-domain": "network_alias = { workspace = true }",
                "maestria-governance": 'maestria-domain = { package = "maestria-domain", path = "../../domain" }',
                "maestria-ports": 'maestria-domain = { package = "maestria-domain", path = "../../domain" }',
            }.items():
                crate = root / "crates" / "kernel" / name
                (crate / "src").mkdir(parents=True)
                (crate / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\n[dependencies]\n{dependency}\n',
                    encoding="utf-8",
                )
            violations = PHILOSOPHY_CHECK.scan_kernel_manifests()
            self.assertIn(
                "crates/kernel/maestria-domain/Cargo.toml contains forbidden dependency token reqwest",
                violations,
            )

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
            old_maps = PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS
            PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = {}
            try:
                self.assertEqual(
                    PHILOSOPHY_CHECK.scan_responsibility_maps(),
                    ["external/example/src/lib.rs production module has no configured responsibility map"],
                )
            finally:
                PHILOSOPHY_CHECK.RESPONSIBILITY_MAPS = old_maps

    def test_facade_boundary_honors_existing_adr_exemption_only_for_named_path(self) -> None:
        old_exemptions = PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = {
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

                self.assertEqual(len(PHILOSOPHY_CHECK.scan_facade_boundaries()), 1)
        finally:
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = old_exemptions

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

    def test_scan_rust_lint_bypasses_reports_expect_attribute(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "#[expect(dead_code)]\nfn example() {}\n", encoding="utf-8"
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_rust_lint_bypasses(),
                ["crates/apps/example/src/lib.rs"],
            )

    def test_scan_function_sizes_reports_oversized_function(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "logic.rs"
            source.parent.mkdir(parents=True)
            body = "\n".join(f"    let _ = {i};" for i in range(101))
            source.write_text(f"pub fn big() {{\n{body}\n}}\n", encoding="utf-8")

            violations = PHILOSOPHY_CHECK.scan_function_sizes()
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

            self.assertEqual(PHILOSOPHY_CHECK.scan_function_sizes(), [])

    def test_scan_function_sizes_respects_exemptions(self) -> None:
        old_exemptions = PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = {
                "crates/apps/example/src/logic.rs": {"big": "v0.9.0"},
            }
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.configure_root(root)
                source = root / "crates" / "apps" / "example" / "src" / "logic.rs"
                source.parent.mkdir(parents=True)
                body = "\n".join(f"    let _ = {i};" for i in range(100))
                source.write_text(f"pub fn big() {{\n{body}\n}}\n", encoding="utf-8")

                self.assertEqual(PHILOSOPHY_CHECK.scan_function_sizes(), [])
        finally:
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = old_exemptions

    def test_function_exemption_is_scoped_to_named_item(self) -> None:
        old_exemptions = PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = {
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

                violations = PHILOSOPHY_CHECK.scan_function_sizes()

                self.assertEqual(len(violations), 1)
                self.assertIn("function `newly_added`", violations[0])
        finally:
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = old_exemptions

    def test_scan_mixed_responsibilities_flags_large_multi_mod_file(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "orchestrator.rs"
            source.parent.mkdir(parents=True)
            lines = ["mod a;", "mod b;", "mod c;"]
            lines.extend(f"pub fn item_{i}() {{}}" for i in range(300))
            source.write_text("\n".join(lines) + "\n", encoding="utf-8")

            violations = PHILOSOPHY_CHECK.scan_mixed_responsibilities()
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

            self.assertEqual(PHILOSOPHY_CHECK.scan_mixed_responsibilities(), [])

    def test_scan_mixed_responsibilities_respects_exemptions(self) -> None:
        old_exemptions = PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = {
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

                self.assertEqual(PHILOSOPHY_CHECK.scan_mixed_responsibilities(), [])
        finally:
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = old_exemptions

    def test_exemption_expiry_covers_function_and_mixed_responsibility_exemptions(
        self,
    ) -> None:
        old_fn = PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS
        old_mixed = PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS
        try:
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = {
                "crates/example/src/large_fn.rs": {"large": "v0.6.0"},
            }
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = {
                "crates/example/src/mixed.rs": "v0.6.0",
            }
            PHILOSOPHY_CHECK.MODULE_SIZE_EXEMPTIONS = {}
            PHILOSOPHY_CHECK.ADR_MODULE_EXEMPTIONS = {}
            violations = PHILOSOPHY_CHECK.scan_exemption_expiry("0.7.0")
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
            PHILOSOPHY_CHECK.FUNCTION_SIZE_EXEMPTIONS = old_fn
            PHILOSOPHY_CHECK.MIXED_RESPONSIBILITY_EXEMPTIONS = old_mixed

    def test_type_invariant_scan_rejects_opposite_boolean_states(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "approval.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Approval<T: Marker<Vec<u8>>> {\n"
                "    pub is_approved: bool, // misleading { and , tokens\n"
                '    #[serde(rename = "denied{,")]\n'
                "    pub denied: bool,\n"
                "    pub marker: PhantomData<T>,\n"
                "}\n"
                "pub const fn resolve(approved: bool, denied: bool) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/approval.rs struct `Approval` "
                    "represents opposite states `approved` and `denied` as booleans; "
                    "use an enum",
                    "crates/kernel/maestria-domain/src/approval.rs function `resolve` "
                    "accepts opposite states `approved` and `denied` as booleans; "
                    "accept an enum",
                ],
            )

    def test_type_invariant_scan_rejects_boolean_with_optional_state_payload(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "job.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Job { pub failed: bool, pub error: Option<String> }\n"
                "pub fn finish(failed: bool, error: Option<String>) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/job.rs struct `Job` coordinates "
                    "boolean state `failed` with optional payload `error`; put the "
                    "payload on an enum variant",
                    "crates/kernel/maestria-domain/src/job.rs function `finish` "
                    "coordinates boolean state `failed` with optional payload "
                    "`error`; accept an enum carrying the payload",
                ],
            )

    def test_type_invariant_scan_rejects_stringly_state(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "task.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Task { pub status: String, pub title: String }\n"
                "pub const fn transition(status: &str) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/task.rs struct `Task` "
                    "represents state field `status` as `String`; use an enum or "
                    "validated domain type",
                    "crates/kernel/maestria-domain/src/task.rs function `transition` "
                    "accepts state parameter `status` as `&str`; accept an enum or "
                    "validated domain type",
                ],
            )

    def test_type_invariant_scan_rejects_swappable_primitive_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "relation.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct Relation { pub source_id: u64, pub target_id: u64 }\n"
                "pub fn connect(parent_id: u64, child_id: u64) {}\n"
                'extern "C" fn link(left_id: u64, right_id: u64) {}\n',
                encoding="utf-8",
            )

            self.assertEqual(
                PHILOSOPHY_CHECK.scan_type_invariant_modeling(),
                [
                    "crates/kernel/maestria-domain/src/relation.rs struct `Relation` "
                    "has swappable primitive identities `source_id`, `target_id` of "
                    "type `u64`; use distinct ID types",
                    "crates/kernel/maestria-domain/src/relation.rs function `connect` "
                    "accepts swappable primitive identities `parent_id`, `child_id` "
                    "of type `u64`; use distinct ID types",
                    "crates/kernel/maestria-domain/src/relation.rs function `link` "
                    "accepts swappable primitive identities `left_id`, `right_id` "
                    "of type `u64`; use distinct ID types",
                ],
            )

    def test_type_invariant_scan_accepts_independent_flags_and_typed_ids(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "search.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "pub struct QueryId(u64);\n"
                "pub struct CorpusId(u64);\n"
                "pub struct SearchOptions {\n"
                "    pub include_archived: bool,\n"
                "    pub preserve_seed: bool,\n"
                "    pub query_hint: Option<String>,\n"
                "}\n"
                "pub enum SearchState { Planned, Running, Complete }\n"
                "pub fn search(query_id: QueryId, corpus_id: CorpusId) {}\n"
                "fn format_label(kind: &str) {}\n",
                encoding="utf-8",
            )

            self.assertEqual(PHILOSOPHY_CHECK.scan_type_invariant_modeling(), [])


if __name__ == "__main__":
    unittest.main()
