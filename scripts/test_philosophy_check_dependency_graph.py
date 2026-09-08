"""Tests for the philosophy_check dependency graph family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import dependency_graph, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class DependencyGraphTests(PhilosophyCheckFixture):

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
            gov = root / "crates" / "kernel" / "maestria-governance"
            gov.mkdir(parents=True)
            (gov / "Cargo.toml").write_text('[package]\nname = "maestria-governance"\n', encoding="utf-8")
            ports = root / "crates" / "kernel" / "maestria-ports"
            ports.mkdir(parents=True)
            (ports / "Cargo.toml").write_text('[package]\nname = "maestria-ports"\n', encoding="utf-8")
            source.write_text(
                "use std::fs;\n"
                'pub fn production_failure() { panic!("forbidden"); }\n'
                "#[cfg(test)]\n"
                "mod tests { fn test_only() { value.unwrap(); } }\n",
                encoding="utf-8",
            )

            manifest_violations = dependency_graph.scan_kernel_manifests()
            source_violations = dependency_graph.scan_kernel_sources()

            self.assertEqual(
                manifest_violations,
                [
                    "crates/kernel/maestria-domain/Cargo.toml contains forbidden dependency token tokio"
                ],
            )
            self.assertIn(
                "crates/kernel/maestria-domain/src/lib.rs contains forbidden kernel token std::fs",
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
            dependency_graph._manifest_dependencies(content),
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

            violations = dependency_graph.scan_kernel_manifests()

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
            self.assertEqual(dependency_graph.scan_kernel_manifests(), [])

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
                dependency_graph.scan_kernel_manifests(),
                [
                    "crates/kernel/maestria-governance/Cargo.toml "
                    "contains forbidden dependency token reqwest"
                ],
            )
            self.assertEqual(
                dependency_graph.scan_kernel_sources(),
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
                dependency_graph.scan_kernel_sources(),
                [
                    "crates/kernel/maestria-domain/src/network.rs contains "
                    "forbidden kernel token std::net",
                    "crates/kernel/maestria-domain/src/network.rs contains "
                    "forbidden unsafe Rust",
                ],
            )

    def test_facade_boundary_discovers_unlisted_lib_rs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            lib_dir = root / "crates" / "ecosystem" / "example" / "src"
            lib_dir.mkdir(parents=True)
            (lib_dir / "lib.rs").write_text(
                "pub fn unlisted_helper() -> i32 { 42 }\n", encoding="utf-8"
            )

            violations = dependency_graph.scan_facade_boundaries()

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

            self.assertEqual(dependency_graph.scan_facade_boundaries(), [])

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
            violations = dependency_graph.scan_facade_boundaries()
            self.assertEqual(len(violations), 1)
            self.assertIn("8 implementation item(s)", violations[0])

    def test_facade_boundary_rejects_whitespace_separated_wildcard(self) -> None:
        self.assertFalse(
            dependency_graph._facade_item_allowed("pub use x::\n\t *;")
        )

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
            violations = dependency_graph.scan_kernel_manifests()
            self.assertIn(
                "crates/kernel/maestria-domain/Cargo.toml contains forbidden dependency token reqwest",
                violations,
            )

    def test_kernel_import_scan_rejects_non_kernel_import(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                "use maestria_storage_sqlite::SqliteStore;\n", encoding="utf-8"
            )

            self.assertEqual(
                dependency_graph.scan_kernel_imports(),
                [
                    "crates/kernel/maestria-domain/src/lib.rs "
                    "imports forbidden kernel dependency maestria_storage_sqlite"
                ],
            )

    def test_kernel_import_scan_accepts_declared_kernel_dependencies(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            governance = (
                root / "crates" / "kernel" / "maestria-governance" / "src" / "lib.rs"
            )
            governance.parent.mkdir(parents=True)
            governance.write_text(
                "use maestria_domain::KernelState;\n"
                "use maestria_domain as domain;\n",
                encoding="utf-8",
            )

            self.assertEqual(dependency_graph.scan_kernel_imports(), [])

    def test_kernel_import_scan_ignores_comments_and_strings(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            )
            source.parent.mkdir(parents=True)
            source.write_text(
                '// use maestria_tantivy::TantivyFullTextIndex;\n'
                'const EXAMPLE: &str = "use maestria_tantivy::Index";\n',
                encoding="utf-8",
            )

            self.assertEqual(dependency_graph.scan_kernel_imports(), [])

    def test_kernel_tokens_reject_random_sampling(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("fn pick() { let _ = thread_rng(); }\n")
            violations = dependency_graph.scan_kernel_sources()
            self.assertTrue(any("thread_rng" in item for item in violations))

    def test_dependency_closure_skips_without_cargo_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            violations = dependency_graph.scan_kernel_dependency_closure()
            self.assertEqual(violations, [])

    def test_kernel_tokens_reject_maybe_uninit(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("fn init() { let _ = std::mem::MaybeUninit::<u8>::uninit(); }\n")
            violations = dependency_graph.scan_kernel_sources()
            self.assertTrue(any("MaybeUninit" in item for item in violations), violations)
