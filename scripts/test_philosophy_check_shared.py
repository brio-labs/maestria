"""Tests for the philosophy_check shared family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import panic_and_lint, secrets, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class SharedHelpersTests(PhilosophyCheckFixture):

    def test_rust_inventory_keeps_live_sources_and_excludes_build_trees(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            names = (
                "src/lib.rs",
                "src/nested/worker.rs",
                "tests/behavior.rs",
                "target/generated.rs",
                "src/target/hidden.rs",
                "node_modules/dependency.rs",
            )
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fn sample() {}\n", encoding="utf-8")
            inventory = {
                path.relative_to(root).as_posix()
                for path in shared._production_rust_files()
            }
            self.assertEqual(inventory, {"src/lib.rs", "src/nested/worker.rs"})
            complete = {
                path.relative_to(root).as_posix()
                for path in shared._production_rust_files(skip_tests=False)
            }
            self.assertEqual(complete, inventory | {"tests/behavior.rs"})
            added = root / "src/added.rs"
            added.write_text("fn later() {}\n", encoding="utf-8")
            self.assertIn(added, tuple(shared._production_rust_files()))

    def test_external_patches_keep_secret_scanning_and_first_party_doctrine(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["member"]\n'
                'exclude = ["vendor/dependency"]\n\n'
                '[patch.crates-io]\n'
                'dependency = { path = "vendor/dependency" }\n'
                'member = { path = "member" }\n',
                encoding="utf-8",
            )
            for name in ("member", "vendor/dependency", "vendor/neighbor"):
                source = root / name / "src" / "lib.rs"
                source.parent.mkdir(parents=True)
                source.write_text("fn example() { Some(1).unwrap(); }\n", encoding="utf-8")
            upstream = root / "vendor/dependency/src/lib.rs"
            with upstream.open("a", encoding="utf-8") as stream:
                stream.write(f'const TOKEN: &str = "ghp_{"0" * 36}";\n')

            violations = panic_and_lint.scan_rust_forbidden_methods()
            self.assertEqual(
                {violation.split(" contains ", 1)[0] for violation in violations},
                {"member/src/lib.rs", "vendor/neighbor/src/lib.rs"},
            )
            findings = secrets.scan_hardcoded_secrets()
            self.assertEqual(len(findings), 1)
            self.assertIn("vendor/dependency/src/lib.rs", findings[0])

    def test_launcher_source_checks_generated_context_lint_exemptions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates/apps/sillage-launcher/src/lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text(
                "use std::collections::HashMap;\n"
                "use std::time::Instant;\n"
                "fn sample() { let _: HashMap<u8, u8> = Default::default(); "
                "let _ = Instant::now(); }\n",
                encoding="utf-8",
            )
            violations = panic_and_lint.scan_rust_forbidden_methods()
            self.assertTrue(any("forbidden hash collection type" in item for item in violations))
            self.assertTrue(any("forbidden wall-clock instant" in item for item in violations))

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

    def test_test_source_classification_follows_module_reachability(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            source = root / "src"
            files = {
                "lib.rs": (
                    "mod shipped_tests;\n"
                    "mod test_support;\n"
                    "mod production_tests;\n"
                    "mod tests;\n"
                    "mod tests_boundary;\n"
                    "mod path_owner;\n"
                    "mod watcher_tests {\n    mod e2e;\n}\n"
                    "mod contract_tests {\n    mod misc;\n}\n"
                    "#[cfg(test)]\nmod unrelated_sibling;\n"
                    "#[cfg(test)]\nmod directory_module;\n"
                    "#[cfg(test)]\n"
                    '#[path = "actual/test_helper.rs"]\nmod test_helper;\n'
                    "#[path = \"mixed.rs\"]\nmod mixed_production;\n"
                    "#[cfg(test)]\n"
                    '#[path = "mixed.rs"]\nmod mixed_test_copy;\n'
                    "#[cfg(feature = \"optional\")]\nmod feature_tests;\n"
                    "#[cfg(any(test, feature = \"shipping\"))]\nmod either_tests;\n"
                    "#[cfg(test)]\n"
                    '#[cfg_attr(test, path = "conditional.rs")]\nmod conditional;\n'
                    "#[cfg(test)]\ninclude!(\"included_tests.rs\");\n"
                    "/* #[cfg(test)] #[path = \"fake_comment.rs\"] mod fake_comment; */\n"
                    'const FAKE: &str = "#[cfg(test)] #[path = \\"fake_literal.rs\\"] mod fake_literal;";\n'
                    "#[cfg(test)]\n"
                    '#[path = concat!("unsupported", ".rs")]\nmod unsupported;\n'
                ),
                "shipped_tests.rs": "fn production_reachable() {}\n",
                "path_owner.rs": (
                    "#[cfg(test)]\n"
                    '#[path = "fixture.rs"]\nmod fixture;\n'
                    "fn production_reachable() {}\n"
                ),
                "fixture.rs": (
                    '#[path = "fixture_helpers/provider.rs"]\n'
                    "mod provider;\n"
                ),
                "fixture_helpers/provider.rs": "fn test_only() {}\n",
                "test_support.rs": "fn production_reachable() {}\n",
                "production_tests.rs": "fn production_reachable() {}\n",
                "tests.rs": "fn production_reachable() {}\n",
                "tests_boundary.rs": "fn production_reachable() {}\n",
                "watcher_tests/e2e.rs": "fn production_reachable() {}\n",
                "contract_tests/misc.rs": "fn production_reachable() {}\n",
                "directory_module/mod.rs": "mod nested_directory;\n",
                "directory_module/nested_directory/mod.rs": "fn test_only() {}\n",
                "unrelated_sibling.rs": (
                    "mod nested;\n"
                    "fn test_only() { let _ = Some(1).unwrap(); }\n"
                ),
                "unrelated_sibling/nested.rs": "mod deeper;\n",
                "unrelated_sibling/nested/deeper.rs": "fn test_only() {}\n",
                "actual/test_helper.rs": "mod leaf;\n",
                "actual/test_helper/leaf.rs": "fn test_only() {}\n",
                "mixed.rs": "mod descendant;\n",
                "mixed/descendant.rs": "fn mixed_reachability() {}\n",
                "feature_tests.rs": "fn condition_is_not_test_only() {}\n",
                "either_tests.rs": "fn condition_is_not_test_only() {}\n",
                "conditional.rs": "fn cfg_attr_path_is_not_guessed() {}\n",
                "included_tests.rs": "fn include_target_is_not_assumed_test_only() {}\n",
                "unsupported_tests.rs": "fn unsupported_path_is_not_guessed() {}\n",
                "fake_comment.rs": "fn comment_did_not_declare_this() {}\n",
                "fake_literal.rs": "fn literal_did_not_declare_this() {}\n",
            }
            for relative, content in files.items():
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")
            integration_test = root / "tests" / "behavior.rs"
            integration_test.parent.mkdir()
            integration_test.write_text("fn integration_test() {}\n", encoding="utf-8")

            def test_only(relative: str) -> bool:
                return shared.is_test_source(source / relative)

            for relative in (
                "unrelated_sibling.rs",
                "unrelated_sibling/nested.rs",
                "unrelated_sibling/nested/deeper.rs",
                "actual/test_helper.rs",
                "fixture.rs",
                "fixture_helpers/provider.rs",
                "actual/test_helper/leaf.rs",
                "directory_module/mod.rs",
                "directory_module/nested_directory/mod.rs",
            ):
                with self.subTest(relative=relative):
                    self.assertTrue(test_only(relative))
            for relative in (
                "shipped_tests.rs",
                "test_support.rs",
                "production_tests.rs",
                "tests.rs",
                "path_owner.rs",
                "tests_boundary.rs",
                "watcher_tests/e2e.rs",
                "contract_tests/misc.rs",
                "mixed.rs",
                "mixed/descendant.rs",
                "feature_tests.rs",
                "either_tests.rs",
                "conditional.rs",
                "included_tests.rs",
                "unsupported_tests.rs",
                "fake_comment.rs",
                "fake_literal.rs",
            ):
                with self.subTest(relative=relative):
                    self.assertFalse(test_only(relative))
            self.assertTrue(shared.is_test_source(integration_test))
            violations = panic_and_lint.scan_rust_forbidden_methods()
            self.assertTrue(
                any(
                    item.startswith("src/unrelated_sibling.rs contains ")
                    for item in violations
                )
            )

    def test_conjunctive_cfg_test_modules_are_test_only_but_any_feature_is_shipped(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n\n'
                '[features]\nshipping = []\n',
                encoding="utf-8",
            )
            source = root / "src"
            source.mkdir()
            (source / "lib.rs").write_text(
                "#[cfg(any(all(test, target_os = \"linux\"), "
                "all(test, target_os = \"windows\")))]\n"
                '#[path = "lock_tests.rs"]\n'
                "mod lock_tests;\n"
                "#[cfg(any(test, feature = \"shipping\"))]\n"
                '#[path = "shipping.rs"]\n'
                "mod shipping;\n",
                encoding="utf-8",
            )
            test_source = source / "lock_tests.rs"
            test_source.write_text(
                "#[test]\n"
                "fn lock_invariant() { assert!(true); }\n",
                encoding="utf-8",
            )
            shipping_source = source / "shipping.rs"
            shipping_source.write_text(
                "pub fn shipping_invariant() { assert!(true); }\n",
                encoding="utf-8",
            )

            inline_production = shared.production_rust(
                "#[cfg(any(all(test, target_os = \"linux\"), "
                "all(test, target_os = \"windows\")))]\n"
                "mod tests { fn check() { assert!(true); } }\n"
                "pub fn shipped() {}\n"
            )
            self.assertNotIn("assert!", inline_production)
            self.assertIn("pub fn shipped", inline_production)

            self.assertTrue(shared.is_test_source(test_source))
            self.assertFalse(shared.is_test_source(shipping_source))
            violations = panic_and_lint.scan_production_asserts()
            self.assertEqual(len(violations), 1, violations)
            self.assertTrue(
                violations[0].startswith("src/shipping.rs:1"),
                violations,
            )

    def test_explicit_pathless_test_targets_are_test_only_with_autotests_disabled(
        self,
    ) -> None:
        for relative_test, helper_path in (
            ("tests/behavior.rs", "../src/fixture.rs"),
            ("tests/behavior/main.rs", "../../src/fixture.rs"),
        ):
            with self.subTest(relative_test=relative_test):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    self.configure_root(root)
                    (root / "Cargo.toml").write_text(
                        '[package]\nname = "fixture"\nversion = "0.1.0"\n'
                        "autotests = false\n\n"
                        "[[test]]\nname = \"behavior\"\n",
                        encoding="utf-8",
                    )
                    library = root / "src" / "lib.rs"
                    library.parent.mkdir(parents=True)
                    library.write_text("", encoding="utf-8")
                    test_source = root / relative_test
                    test_source.parent.mkdir(parents=True)
                    test_source.write_text(
                        f'#[path = "{helper_path}"]\n'
                        "mod fixture;\n"
                        "#[test]\n"
                        "fn explicit_target_is_present() {}\n",
                        encoding="utf-8",
                    )
                    helper = root / "src" / "fixture.rs"
                    helper.write_text(
                        "pub fn fixture_check() { assert!(true); }\n",
                        encoding="utf-8",
                    )

                    self.assertTrue(shared.is_test_source(test_source))
                    self.assertTrue(shared.is_test_source(helper))
                    self.assertEqual(
                        panic_and_lint.scan_production_asserts(), []
                    )

    def test_disabled_automatic_shipping_roots_do_not_override_test_routes(
        self,
    ) -> None:
        disabled_roots = (
            ("autobins = false", "src/main.rs", "helper.rs"),
            ("autoexamples = false", "examples/example.rs", "../src/helper.rs"),
            ("autobenches = false", "benches/benchmark.rs", "../src/helper.rs"),
            ("build = false", "build.rs", "src/helper.rs"),
        )
        for package_flag, relative_root, helper_path in disabled_roots:
            with self.subTest(package_flag=package_flag):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    self.configure_root(root)
                    (root / "Cargo.toml").write_text(
                        '[package]\nname = "fixture"\nversion = "0.1.0"\n'
                        f"{package_flag}\n",
                        encoding="utf-8",
                    )
                    library = root / "src" / "lib.rs"
                    library.parent.mkdir(parents=True)
                    library.write_text("", encoding="utf-8")
                    helper = root / "src" / "helper.rs"
                    helper.write_text(
                        "pub fn fixture_check() { assert!(true); }\n",
                        encoding="utf-8",
                    )
                    disabled_root = root / relative_root
                    disabled_root.parent.mkdir(parents=True, exist_ok=True)
                    disabled_root.write_text(
                        f'#[path = "{helper_path}"]\nmod helper;\n',
                        encoding="utf-8",
                    )
                    test_root = root / "tests" / "behavior.rs"
                    test_root.parent.mkdir(parents=True)
                    test_root.write_text(
                        '#[path = "../src/helper.rs"]\nmod helper;\n'
                        "#[test]\nfn fixture_route_is_enabled() {}\n",
                        encoding="utf-8",
                    )

                    self.assertTrue(shared.is_test_source(helper))
                    self.assertEqual(
                        panic_and_lint.scan_production_asserts(), []
                    )

    def test_explicit_shipping_target_remains_active_when_autobins_is_false(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n'
                "autobins = false\n\n"
                "[[bin]]\nname = \"explicit\"\npath = \"src/bin/explicit.rs\"\n",
                encoding="utf-8",
            )
            library = root / "src" / "lib.rs"
            library.parent.mkdir(parents=True)
            library.write_text("", encoding="utf-8")
            helper = root / "src" / "helper.rs"
            helper.write_text(
                "pub fn fixture_check() { assert!(true); }\n",
                encoding="utf-8",
            )
            explicit_bin = root / "src" / "bin" / "explicit.rs"
            explicit_bin.parent.mkdir(parents=True)
            explicit_bin.write_text(
                '#[path = "../helper.rs"]\nmod helper;\n',
                encoding="utf-8",
            )
            test_root = root / "tests" / "behavior.rs"
            test_root.parent.mkdir(parents=True)
            test_root.write_text(
                '#[path = "../src/helper.rs"]\nmod helper;\n'
                "#[test]\nfn fixture_route_is_enabled() {}\n",
                encoding="utf-8",
            )

            self.assertFalse(shared.is_test_source(helper))
            violations = panic_and_lint.scan_production_asserts()
            self.assertEqual(len(violations), 1, violations)
            self.assertTrue(
                violations[0].startswith("src/helper.rs:1"),
                violations,
            )

    def test_pathless_explicit_shipping_targets_survive_disabled_autotargets(
        self,
    ) -> None:
        targets = (
            ("bin", "autobins = false", "src/bin/shipping.rs", "../helper.rs"),
            (
                "example",
                "autoexamples = false",
                "examples/shipping.rs",
                "../src/helper.rs",
            ),
            (
                "bench",
                "autobenches = false",
                "benches/shipping.rs",
                "../src/helper.rs",
            ),
        )
        for target_kind, package_flag, relative_root, helper_path in targets:
            with self.subTest(target_kind=target_kind):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    self.configure_root(root)
                    (root / "Cargo.toml").write_text(
                        '[package]\nname = "fixture"\nversion = "0.1.0"\n'
                        f"{package_flag}\n\n"
                        f"[[{target_kind}]]\nname = \"shipping\"\n",
                        encoding="utf-8",
                    )
                    library = root / "src" / "lib.rs"
                    library.parent.mkdir(parents=True)
                    library.write_text("", encoding="utf-8")
                    helper = root / "src" / "helper.rs"
                    helper.write_text(
                        "pub fn fixture_check() { assert!(true); }\n",
                        encoding="utf-8",
                    )
                    shipping_root = root / relative_root
                    shipping_root.parent.mkdir(parents=True)
                    shipping_root.write_text(
                        f'#[path = "{helper_path}"]\nmod helper;\n',
                        encoding="utf-8",
                    )
                    test_root = root / "tests" / "behavior.rs"
                    test_root.parent.mkdir(parents=True)
                    test_root.write_text(
                        '#[path = "../src/helper.rs"]\nmod helper;\n'
                        "#[test]\nfn fixture_route_is_enabled() {}\n",
                        encoding="utf-8",
                    )

                    self.assertFalse(shared.is_test_source(helper))
                    violations = panic_and_lint.scan_production_asserts()
                    self.assertEqual(len(violations), 1, violations)
                    self.assertTrue(
                        violations[0].startswith("src/helper.rs:1"),
                        violations,
                    )

    def test_inline_path_directories_keep_suffixes_when_resolving_children(
        self,
    ) -> None:
        for inline_directory, misplaced_directory in (
            ("fixtures.v1", "fixtures"),
            ("mod.rs", None),
        ):
            with self.subTest(inline_directory=inline_directory):
                with tempfile.TemporaryDirectory() as tmp:
                    root = Path(tmp)
                    self.configure_root(root)
                    (root / "Cargo.toml").write_text(
                        '[package]\nname = "fixture"\nversion = "0.1.0"\n',
                        encoding="utf-8",
                    )
                    source = root / "src"
                    source.mkdir()
                    (source / "lib.rs").write_text(
                        "#[cfg(test)]\n"
                        f'#[path = "{inline_directory}/leaf.rs"]\n'
                        "mod test_leaf;\n"
                        f'#[path = "{inline_directory}"]\n'
                        "mod nested {\n"
                        "    mod leaf;\n"
                        "}\n",
                        encoding="utf-8",
                    )
                    leaf = source / inline_directory / "leaf.rs"
                    leaf.parent.mkdir(parents=True)
                    leaf.write_text(
                        "pub fn production_check() { assert!(true); }\n",
                        encoding="utf-8",
                    )
                    misplaced_leaf = (
                        source / "leaf.rs"
                        if misplaced_directory is None
                        else source / misplaced_directory / "leaf.rs"
                    )
                    misplaced_leaf.parent.mkdir(parents=True, exist_ok=True)
                    misplaced_leaf.write_text(
                        "pub fn wrong_path_decoy() {}\n",
                        encoding="utf-8",
                    )

                    self.assertFalse(shared.is_test_source(leaf))
                    violations = panic_and_lint.scan_production_asserts()
                    self.assertEqual(len(violations), 1, violations)
                    self.assertTrue(
                        violations[0].startswith(
                            f"src/{inline_directory}/leaf.rs:1"
                        ),
                        violations,
                    )

    def test_inner_cfg_test_guards_the_whole_module_file_and_children(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            source = root / "src"
            source.mkdir()
            (source / "lib.rs").write_text(
                "mod tests;\n"
                "pub fn production_check() { assert!(true); }\n",
                encoding="utf-8",
            )
            tests_module = source / "tests.rs"
            tests_module.write_text(
                "#![cfg(test)]\n"
                "pub fn test_fixture() { assert!(true); }\n"
                "mod nested;\n",
                encoding="utf-8",
            )
            nested_module = source / "tests" / "nested.rs"
            nested_module.parent.mkdir()
            nested_module.write_text(
                "pub fn nested_fixture() { assert!(true); }\n",
                encoding="utf-8",
            )

            self.assertTrue(shared.is_test_source(tests_module))
            self.assertTrue(shared.is_test_source(nested_module))
            production_test_module = shared.production_rust(
                tests_module.read_text(encoding="utf-8")
            )
            self.assertNotIn("assert!", production_test_module)
            self.assertEqual(production_test_module.count("\n"), 3)
            inline_scope = shared.production_rust(
                "mod inner {\n"
                "    #![cfg(test)]\n"
                "    pub fn only_test() { assert!(true); }\n"
                "}\n"
                "pub fn shipping() {}\n"
            )
            self.assertNotIn("assert!", inline_scope)
            self.assertIn("pub fn shipping", inline_scope)

            violations = panic_and_lint.scan_production_asserts()
            self.assertEqual(len(violations), 1, violations)
            self.assertTrue(
                violations[0].startswith("src/lib.rs:2"),
                violations,
            )

    def test_shipping_routes_keep_test_reachable_sources_production(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n\n'
                '[features]\nshipping = []\nalt = []\n',
                encoding="utf-8",
            )
            source = root / "src"
            files = {
                "lib.rs": (
                    "#[cfg(test)]\n"
                    '#[path = "include_shared.rs"]\nmod include_tests;\n'
                    'include!("include_shared.rs");\n'
                    "#[cfg(test)]\n"
                    '#[path = "local_shared.rs"]\nmod local_tests;\n'
                    "fn production_function() {\n"
                    '    #[path = "local_shared.rs"]\n'
                    "    mod local_route;\n"
                    "}\n"
                    "#[cfg(test)]\n"
                    '#[path = "conditional_shared.rs"]\nmod conditional_tests;\n'
                    "#[cfg_attr(feature = \"shipping\", "
                    'path = "conditional_shared.rs")]\n'
                    "mod conditional_shipping;\n"
                    "#[cfg(test)]\n"
                    '#[path = "method_shared.rs"]\nmod method_tests;\n'
                    "struct ShippingHost;\n"
                    "impl ShippingHost {\n"
                    "    fn start() {\n"
                    '        #[path = "method_shared.rs"]\n'
                    "        mod method_route;\n"
                    "    }\n"
                    "}\n"
                    "#[cfg(test)]\n"
                    '#[path = "block_shared.rs"]\nmod block_tests;\n'
                    "fn shipping_nested_block() {\n"
                    "    if true {\n"
                    '        #[path = "block_shared.rs"]\n'
                    "        mod block_route;\n"
                    "    }\n"
                    "}\n"
                    "#[cfg(test)]\n"
                    '#[path = "closure_shared.rs"]\nmod closure_tests;\n'
                    "fn shipping_closure() {\n"
                    "    let _closure = || {\n"
                    '        #[path = "closure_shared.rs"]\n'
                    "        mod closure_route;\n"
                    "    };\n"
                    "}\n"
                    "#[cfg(test)]\n"
                    '#[path = "default_shared.rs"]\nmod default_tests;\n'
                    "#[cfg_attr(feature = \"alt\", "
                    'path = "alternate_shared.rs")]\n'
                    "mod default_shared;\n"
                    "#[cfg(test)]\n"
                    '#[path = "macro_shared.rs"]\nmod macro_tests;\n'
                    "macro_rules! unexpanded_module {\n"
                    "    () => {\n"
                    '        #[path = "macro_shared.rs"]\n'
                    "        mod unexpanded_module;\n"
                    "    };\n"
                    "}\n"
                ),
                "method_shared.rs": "fn method() {}\n",
                "block_shared.rs": "fn block() {}\n",
                "closure_shared.rs": "fn closure() {}\n",
                "default_shared.rs": "fn default_branch() {}\n",
                "alternate_shared.rs": "fn alternate_branch() {}\n",
                "include_shared.rs": "fn included() {}\n",
                "local_shared.rs": "fn local() {}\n",
                "conditional_shared.rs": "fn conditional() {}\n",
                "conditional_shipping.rs": "fn fallback() {}\n",
                "macro_shared.rs": "fn macro_shared() {}\n",
            }
            for relative, content in files.items():
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")

            for relative in (
                "include_shared.rs",
                "local_shared.rs",
                "conditional_shared.rs",
                "conditional_shipping.rs",
                "method_shared.rs",
                "block_shared.rs",
                "closure_shared.rs",
                "default_shared.rs",
                "alternate_shared.rs",
            ):
                with self.subTest(relative=relative):
                    self.assertFalse(shared.is_test_source(source / relative))
            self.assertTrue(shared.is_test_source(source / "macro_shared.rs"))

    def test_opaque_shipping_references_do_not_grant_test_context(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n\n'
                '[features]\nshipping = []\n\n'
                '[workspace]\nmembers = ["crates/isolated"]\n',
                encoding="utf-8",
            )
            source = root / "src"
            files = {
                "lib.rs": (
                    "#[cfg(test)]\n"
                    '#[path = "opaque_include_target.rs"]\n'
                    "mod include_tests;\n"
                    'include!(concat!("opaque_include_", "target.rs"));\n'
                    "#[cfg(test)]\n"
                    '#[path = "opaque_path_target.rs"]\nmod path_tests;\n'
                    "#[cfg_attr(feature = \"shipping\", "
                    'path = concat!("opaque_path_", "target.rs"))]\n'
                    "mod shipping_path;\n"
                ),
                "opaque_include_target.rs": "fn included() {}\n",
                "opaque_path_target.rs": "fn referenced() {}\n",
                "shipping_path.rs": "fn default_path() {}\n",
            }
            for relative, content in files.items():
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content, encoding="utf-8")
            isolated_package = root / "crates" / "isolated"
            (isolated_package / "Cargo.toml").parent.mkdir(
                parents=True, exist_ok=True
            )
            (isolated_package / "Cargo.toml").write_text(
                '[package]\nname = "isolated"\nversion = "0.1.0"\n\n'
                '[features]\ntest-path = []\n',
                encoding="utf-8",
            )
            isolated_source = isolated_package / "src"
            isolated_source.mkdir()
            (isolated_source / "lib.rs").write_text(
                "#[cfg(test)]\n"
                '#[path = "isolated_only.rs"]\nmod isolated_tests;\n'
                "#[cfg(test)]\n"
                '#[path = "opaque_fixture.rs"]\nmod opaque_fixture_tests;\n'
                "#[cfg(test)]\n"
                "#[cfg_attr(feature = \"test-path\", "
                'path = concat!("isolated_", "path.rs"))]\n'
                "mod unresolved_test_path;\n"
                "#[cfg(test)]\n"
                'include!(concat!("isolated_", "unknown.rs"));\n',
                encoding="utf-8",
            )
            (isolated_source / "isolated_only.rs").write_text(
                "fn isolated_test() {}\n",
                encoding="utf-8",
            )
            (isolated_source / "opaque_fixture.rs").write_text(
                'include!(concat!("not_", "resolved.rs"));\n'
                "fn assertion() { assert!(true); }\n",
                encoding="utf-8",
            )

            for relative in (
                "opaque_include_target.rs",
                "opaque_path_target.rs",
            ):
                with self.subTest(relative=relative):
                    self.assertFalse(shared.is_test_source(source / relative))
            self.assertTrue(
                shared.is_test_source(isolated_source / "isolated_only.rs")
            )
            self.assertTrue(
                shared.is_test_source(isolated_source / "opaque_fixture.rs")
            )

    def test_disabled_cargo_test_targets_do_not_exempt_unreachable_sources(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n"
                "autotests = false\n",
                encoding="utf-8",
            )
            library = root / "src" / "lib.rs"
            library.parent.mkdir(parents=True)
            library.write_text("pub fn shipped() {}\n", encoding="utf-8")
            unused_test = root / "tests" / "unused.rs"
            unused_test.parent.mkdir()
            unused_test.write_text("fn uncompiled() {}\n", encoding="utf-8")

            self.assertFalse(shared.is_test_source(unused_test))

    def test_module_context_cache_refreshes_after_source_edits(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            source = root / "src"
            source.mkdir()
            library = source / "lib.rs"
            library.write_text(
                "#[cfg(test)]\nmod helper;\n",
                encoding="utf-8",
            )
            helper = source / "helper.rs"
            helper.write_text("fn helper() {}\n", encoding="utf-8")

            self.assertTrue(shared.is_test_source(helper))
            library.write_text("mod helper;\n", encoding="utf-8")
            self.assertFalse(shared.is_test_source(helper))

    def test_production_rust_preserves_late_diagnostic_line(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            (root / "Cargo.toml").write_text(
                '[package]\nname = "fixture"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )
            source = root / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            content = (
                "#[cfg(test)]\n"
                "mod tests {\n"
                "    fn helper() {\n"
                "        let _ = 1;\n"
                "    }\n"
                "}\n"
                "\n"
                "type Error = String;\n"
            )
            source.write_text(content, encoding="utf-8")

            violations = panic_and_lint.scan_string_typed_errors()
            self.assertEqual(len(violations), 1)
            location, separator, message = violations[0].partition(" ")
            source_path, line = location.rsplit(":", 1)
            self.assertEqual(source_path, "src/lib.rs")
            self.assertEqual(line, "8")
            self.assertTrue(separator)
            self.assertIn("string", message.lower())
            self.assertIn("error", message.lower())
