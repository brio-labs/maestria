"""Tests for the philosophy_check shared family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import shared
from philosophy_check_testbase import PhilosophyCheckFixture


class SharedHelpersTests(PhilosophyCheckFixture):

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
                for path in shared.production_lib_paths()
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
                for path in shared.production_lib_paths()
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
                for path in shared.production_lib_paths()
            }
            self.assertEqual(discovered, {"member/src/lib.rs", "implicit/src/lib.rs"})

    def test_module_scanner_handles_same_line_attributes_without_nested_matches(self) -> None:
        modules = shared._top_level_module_declarations(
            "#[cfg(test)] mod tests; "
            "#[cfg(any(test, feature = \"shipping\"))] mod shipping; "
            "pub\nmod split; "
            "mod outer { mod nested; }\n"
        )
        self.assertEqual(modules, {"shipping", "split"})

    def test_production_strip_line_comments_keeps_doc_comments(self) -> None:
        body = "//! doc comment\n// normal comment\npub fn foo() {}\n"
        result = shared._rust_syntax(body)
        self.assertNotIn("// normal comment", result)
        self.assertIn("pub fn foo", result)

    def test_production_rust_keeps_code_after_top_test_import(self) -> None:
        body = (
            "#[cfg(test)]\n"
            "use std::path::PathBuf;\n"
            "use std::collections::HashMap;\n"
            "\n"
            "pub fn f() -> u32 { 1 }\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn t() {}\n"
            "}\n"
        )
        result = shared.production_rust(body)
        self.assertIn("pub fn f", result)
        self.assertIn("use std::collections::HashMap", result)
        self.assertNotIn("use std::path::PathBuf", result)
        self.assertNotIn("mod tests", result)

    def test_production_rust_ignores_literal_in_strings_and_comments(self) -> None:
        body = (
            'pub fn f() -> &\'static str { "#[cfg(test)]" }\n'
            "// docs mention #[cfg(test)] here\n"
            "pub fn g() {}\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {}\n"
        )
        result = shared.production_rust(body)
        self.assertIn('"#[cfg(test)]"', result)
        self.assertIn("// docs mention #[cfg(test)] here", result)
        self.assertIn("pub fn g", result)
        self.assertNotIn("mod tests", result)

    def test_production_rust_ignores_literal_in_raw_string(self) -> None:
        body = 'pub fn f() -> &str { r#"#[cfg(test)]"# }\n\n#[cfg(test)]\nmod tests {}\n'
        result = shared.production_rust(body)
        self.assertIn('r#"#[cfg(test)]"#', result)
        self.assertNotIn("mod tests", result)

    def test_production_rust_strips_gated_function_with_attribute_chain(self) -> None:
        body = (
            "pub fn a() {}\n"
            "\n"
            "#[cfg(test)]\n"
            "#[derive(Debug)]\n"
            "fn helper() -> u32 { 42 }\n"
            "\n"
            "pub fn b() {}\n"
        )
        result = shared.production_rust(body)
        self.assertIn("pub fn a", result)
        self.assertIn("pub fn b", result)
        self.assertNotIn("helper", result)

    def test_production_rust_strips_gated_module_declaration(self) -> None:
        body = "pub fn e() {}\n\n#[cfg(test)]\nmod tests;\n"
        result = shared.production_rust(body)
        self.assertIn("pub fn e", result)
        self.assertNotIn("mod tests", result)

    def test_production_rust_strips_gated_blocks_with_braces_in_strings(self) -> None:
        body = (
            "pub fn d() { let s = \"}\"; }\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    fn t() { let x = \"{\"; assert_eq!(x, \"{\"); }\n"
            "}\n"
        )
        result = shared.production_rust(body)
        self.assertIn("pub fn d", result)
        self.assertNotIn("mod tests", result)

    def test_production_rust_strips_doc_commented_gated_item(self) -> None:
        body = (
            "#[cfg(test)]\n"
            "/// helper docs\n"
            "fn helper() {}\n"
            "\n"
            "pub fn f() {}\n"
        )
        result = shared.production_rust(body)
        self.assertIn("pub fn f", result)
        self.assertNotIn("helper", result)

    def test_production_rust_without_cfg_test_is_unchanged(self) -> None:
        body = "pub fn f() {}\n// plain comment\n"
        self.assertEqual(shared.production_rust(body), body)

    def test_logical_line_count_excludes_block_comments(self) -> None:
        content = (
            "fn f() {\n"
            "    /*\n"
            "    block comment\n"
            "    spans three lines\n"
            "    */\n"
            "    let x = 1; // trailing\n"
            "}\n"
        )
        self.assertEqual(shared.logical_line_count(content), 3)

    def test_is_test_source_covers_test_prefixed_modules(self) -> None:
        self.assertTrue(shared.is_test_source(Path("src/tests_boundary.rs")))
        self.assertTrue(shared.is_test_source(Path("src/watcher_tests/e2e.rs")))
        self.assertTrue(shared.is_test_source(Path("src/test_support.rs")))
        self.assertTrue(shared.is_test_source(Path("src/contract_tests/misc.rs")))
        self.assertFalse(shared.is_test_source(Path("src/lib.rs")))
