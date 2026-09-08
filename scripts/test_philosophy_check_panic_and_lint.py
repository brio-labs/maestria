"""Tests for the philosophy_check panic and lint family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import panic_and_lint, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class PanicAndLintTests(PhilosophyCheckFixture):

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
                panic_and_lint.scan_markers(),
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
                panic_and_lint.scan_rust_lint_bypasses(),
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
                panic_and_lint.scan_rust_lint_bypasses(),
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
                panic_and_lint.scan_rust_forbidden_methods(),
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
                panic_and_lint.scan_unbounded_channels(),
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
                panic_and_lint.scan_unbounded_channels(),
                [
                    "crates/apps/example/src/channels.rs contains an unbounded internal channel"
                ],
            )

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
                panic_and_lint.scan_rust_lint_bypasses(),
                ["crates/apps/example/src/lib.rs"],
            )

    def test_scan_markers_prunes_skipped_directories_at_walk_time(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            ignored = root / "target" / "debug" / "build" / "dep" / "index.rs"
            ignored.parent.mkdir(parents=True)
            ignored.write_text(
                "// " + "TO" + "DO" + ": never scanned\n", encoding="utf-8"
            )
            scanned = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            scanned.parent.mkdir(parents=True)
            scanned.write_text(
                "// " + "TO" + "DO" + ": scanned\n", encoding="utf-8"
            )

            self.assertEqual(
                panic_and_lint.scan_markers(),
                ["crates/kernel/maestria-domain/src/lib.rs"],
            )

    def test_bypassable_validation_reports_serde_try_from_with_public_fields(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "coverage.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "use serde::{Deserialize, Serialize};\n"
                "#[derive(Debug, Clone, Serialize, Deserialize)]\n"
                '#[serde(try_from = "CoverageDto")]\n'
                "pub struct Coverage {\n"
                "    pub percent_covered: u8,\n"
                "    pub gaps: Vec<String>,\n"
                "}\n"
                "#[derive(Deserialize)]\n"
                "struct CoverageDto {\n"
                "    percent_covered: u8,\n"
                "    gaps: Vec<String>,\n"
                "}\n"
                "impl TryFrom<CoverageDto> for Coverage {\n"
                "    type Error = String;\n"
                "    fn try_from(dto: CoverageDto) -> Result<Self, Self::Error> {\n"
                "        if dto.percent_covered > 100 { return Err(\"out of range\".into()); }\n"
                "        Ok(Self { percent_covered: dto.percent_covered, gaps: dto.gaps })\n"
                "    }\n"
                "}\n"
            )
            violations = panic_and_lint.scan_bypassable_validation()
            self.assertTrue(
                any("struct `Coverage` exposes public fields" in item for item in violations),
                violations,
            )

    def test_bypassable_validation_accepts_private_field_validated_struct(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "coverage.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "use serde::{Deserialize, Serialize};\n"
                "#[derive(Debug, Clone, Serialize, Deserialize)]\n"
                '#[serde(try_from = "CoverageDto")]\n'
                "pub struct Coverage {\n"
                "    percent_covered: u8,\n"
                "}\n"
                "#[derive(Deserialize)]\n"
                "struct CoverageDto {\n"
                "    percent_covered: u8,\n"
                "}\n"
                "impl TryFrom<CoverageDto> for Coverage {\n"
                "    type Error = String;\n"
                "    fn try_from(dto: CoverageDto) -> Result<Self, Self::Error> {\n"
                "        if dto.percent_covered > 100 { return Err(\"out of range\".into()); }\n"
                "        Ok(Self { percent_covered: dto.percent_covered })\n"
                "    }\n"
                "}\n"
                "impl Coverage { pub fn percent_covered(&self) -> u8 { self.percent_covered } }\n"
            )
            violations = panic_and_lint.scan_bypassable_validation()
            self.assertFalse(
                any("struct `Coverage`" in item for item in violations),
                violations,
            )

    def test_bypassable_validation_reports_fallible_constructor_with_public_fields(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "coverage.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "pub struct Coverage {\n"
                "    pub percent_covered: u8,\n"
                "}\n"
                "impl Coverage {\n"
                "    pub fn new(percent_covered: u8) -> Result<Self, &'static str> {\n"
                "        if percent_covered > 100 { return Err(\"out of range\"); }\n"
                "        Ok(Self { percent_covered })\n"
                "    }\n"
                "}\n",
                encoding="utf-8",
            )

            violations = panic_and_lint.scan_bypassable_validation()
            self.assertTrue(
                any("struct `Coverage` exposes public fields" in item for item in violations),
                violations,
            )

    def test_string_typed_errors_reports_bare_string_error_type(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "decode.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "impl TryFrom<Dto> for DomainValue {\n"
                "    type Error = String;\n"
                "    fn try_from(dto: Dto) -> Result<Self, Self::Error> { Ok(Self) }\n"
                "}\n"
            )
            violations = panic_and_lint.scan_string_typed_errors()
            self.assertTrue(any("uses String as a conversion error" in item for item in violations))

    def test_cancellation_docs_requires_public_async_docs(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "pub async fn run() {}\n"
            )
            violations = panic_and_lint.scan_cancellation_docs()
            self.assertTrue(any("`run` is a public async operation" in item for item in violations))

    def test_cancellation_docs_accepts_documented_and_crate_private(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "/// # Cancellation\n"
                "/// Dropping the future aborts the wait.\n"
                "pub async fn run() {}\n"
                "pub(crate) async fn internal() {}\n"
            )
            violations = panic_and_lint.scan_cancellation_docs()
            self.assertEqual(violations, [])

    def test_cancellation_docs_accepts_prose_cancel_mention(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "/// Submit one command.\n"
                "/// Dropping the future does not cancel the server-side command.\n"
                "pub async fn submit() {}\n"
            )
            violations = panic_and_lint.scan_cancellation_docs()
            self.assertEqual(violations, [])

    def test_generated_blobs_reports_production_marker_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            production = root / "crates" / "kernel" / "maestria-domain" / "src" / "gen.rs"
            production.parent.mkdir(parents=True, exist_ok=True)
            production.write_text("// DO NOT EDIT: generated by bindgen\npub fn f() {}\n")
            test_file = root / "crates" / "kernel" / "maestria-domain" / "tests" / "gen_fixture.rs"
            test_file.parent.mkdir(parents=True, exist_ok=True)
            test_file.write_text("// @generated fixture data\n")
            violations = panic_and_lint.scan_generated_blobs()
            self.assertTrue(any("gen.rs" in item for item in violations))
            self.assertFalse(any("gen_fixture.rs" in item for item in violations))

    def test_forbidden_methods_reports_catch_unwind(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("fn swallow() { std::panic::catch_unwind(|| {}); }\n")
            violations = panic_and_lint.scan_rust_forbidden_methods()
            self.assertTrue(any("catch_unwind" in item for item in violations))

    def test_memory_unsafety_markers_reports_transmute(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn reinterpret(value: u32) { let _ = std::mem::transmute::<u32, f32>(value); }\n"
            )
            violations = panic_and_lint.scan_memory_unsafety_markers()
            self.assertTrue(any("transmute call" in item for item in violations), violations)

    def test_memory_unsafety_markers_reports_static_mut(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("static mut COUNTER: u64 = 0;\n")
            violations = panic_and_lint.scan_memory_unsafety_markers()
            self.assertTrue(any("static mut" in item for item in violations), violations)

    def test_memory_unsafety_markers_reports_leaks_even_in_tests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "tests" / "leak.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn fixture() { let _ = Box::leak(Box::new(1)); std::mem::forget(2); }\n"
            )
            violations = panic_and_lint.scan_memory_unsafety_markers()
            self.assertTrue(any("Box::leak" in item for item in violations), violations)
            self.assertTrue(any("mem::forget" in item for item in violations), violations)

    def test_unchecked_apis_report_ub_marker_classes(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn f() { let _ = std::mem::transmute_copy::<u32, u64>(&0u32); }\n"
                "fn g(b: &[u8]) { let _ = unsafe { b.get_unchecked(0) }; }\n"
                "fn h() { let _ = String::from_utf8_unchecked(vec![]); }\n"
            )
            violations = panic_and_lint.scan_unchecked_apis()
            self.assertTrue(any("transmute_copy" in item for item in violations), violations)
            self.assertTrue(any("get_unchecked" in item for item in violations), violations)
            self.assertTrue(
                any("from_utf8_unchecked" in item for item in violations), violations
            )

    def test_unchecked_apis_ignore_string_references(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text('fn f() { let _ = "get_unchecked("; }\n')
            violations = panic_and_lint.scan_unchecked_apis()
            self.assertEqual(violations, [])

    def test_failure_tokens_report_markers_even_in_tests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "tests" / "markers.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn f() { todo!(); unimplemented!(); unreachable!(); }\n"
            )
            violations = panic_and_lint.scan_failure_tokens()
            self.assertTrue(any("todo!" in item for item in violations), violations)
            self.assertTrue(any("unimplemented!" in item for item in violations), violations)
            self.assertTrue(any("unreachable!" in item for item in violations), violations)

    def test_failure_tokens_ignore_string_references(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "tests" / "markers.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text('fn f() { let _ = "unreachable!("; }\n')
            violations = panic_and_lint.scan_failure_tokens()
            self.assertEqual(violations, [])

    def test_process_exit_reports_library_usage_but_allows_apps(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            library = root / "crates" / "core" / "maestria-core" / "src" / "lib.rs"
            library.parent.mkdir(parents=True, exist_ok=True)
            library.write_text("fn f() { std::process::exit(1); }\n")
            app = root / "crates" / "apps" / "maestria-cli" / "src" / "main.rs"
            app.parent.mkdir(parents=True, exist_ok=True)
            app.write_text("fn main() { std::process::exit(1); }\n")
            violations = panic_and_lint.scan_process_exit()
            self.assertTrue(any("process::exit" in item for item in violations), violations)
            self.assertEqual(len(violations), 1, violations)

    def test_env_mutation_reports_set_var_even_in_tests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "tests" / "env.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                'fn fixture() { std::env::set_var("PATH", "/hostile"); }\n'
            )
            violations = panic_and_lint.scan_env_mutation()
            self.assertTrue(any("env::set_var" in item for item in violations), violations)

    def test_env_mutation_reports_remove_var_and_set_current_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn fixture() { env::remove_var(\"K\"); env::set_current_dir(\"/tmp\"); }\n"
            )
            violations = panic_and_lint.scan_env_mutation()
            self.assertTrue(any("env::remove_var" in item for item in violations), violations)
            self.assertTrue(
                any("env::set_current_dir" in item for item in violations), violations
            )

    def test_env_mutation_allows_var_reads(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn fixture() { let _ = std::env::var(\"PATH\"); }\n"
            )
            violations = panic_and_lint.scan_env_mutation()
            self.assertEqual(violations, [])

    def test_debug_output_reports_dbg_even_in_tests(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "tests" / "debug.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("fn trace() { dbg!(1); }\n")
            violations = panic_and_lint.scan_debug_output()
            self.assertTrue(any("dbg!" in item for item in violations), violations)

    def test_debug_output_reports_stdout_in_library_only(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            library = root / "crates" / "core" / "maestria-core" / "src" / "lib.rs"
            library.parent.mkdir(parents=True, exist_ok=True)
            library.write_text("fn emit() { println!(\"hello\"); eprintln!(\"bye\"); }\n")
            app = root / "crates" / "apps" / "maestria-cli" / "src" / "main.rs"
            app.parent.mkdir(parents=True, exist_ok=True)
            app.write_text("fn main() { println!(\"hello\"); }\n")
            violations = panic_and_lint.scan_debug_output()
            self.assertTrue(
                any("println!" in item and "maestria-core" in item for item in violations),
                violations,
            )
            self.assertFalse(any("maestria-cli" in item for item in violations), violations)

    def test_kernel_interior_mutability_reports_mutex_in_domain(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "use std::sync::Mutex;\n" "fn guard(m: Mutex<u64>) { let _ = m; }\n"
            )
            violations = panic_and_lint.scan_kernel_interior_mutability()
            self.assertTrue(any("interior-mutability" in item for item in violations), violations)

    def test_kernel_interior_mutability_exempts_in_memory_ports(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = (
                root / "crates" / "kernel" / "maestria-ports" / "src" / "in_memory" / "store.rs"
            )
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("use std::sync::Mutex;\npub struct Store { inner: Mutex<Vec<u8>> }\n")
            violations = panic_and_lint.scan_kernel_interior_mutability()
            self.assertEqual(violations, [])

    def test_production_asserts_reports_shipped_assert(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "core" / "maestria-core" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text("fn check(value: u8) { assert!(value > 0); }\n")
            violations = panic_and_lint.scan_production_asserts()
            self.assertTrue(any("production assert" in item for item in violations), violations)

    def test_production_asserts_skips_tests_and_debug_assert(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "core" / "maestria-core" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "fn check(value: u8) { debug_assert!(value > 0); }\n"
                "#[cfg(test)]\n"
                "mod tests { fn t() { assert_eq!(1, 1); } }\n"
            )
            test_file = root / "crates" / "core" / "maestria-core" / "tests" / "behavior.rs"
            test_file.parent.mkdir(parents=True, exist_ok=True)
            test_file.write_text("fn t() { assert_ne!(1, 2); }\n")
            violations = panic_and_lint.scan_production_asserts()
            self.assertEqual(violations, [])

    def test_unbounded_channels_reports_async_channel_and_flume(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            first = root / "crates" / "apps" / "example" / "src" / "async.rs"
            first.parent.mkdir(parents=True, exist_ok=True)
            first.write_text(
                "fn chans() { let (_tx, _rx) = async_channel::unbounded(); }\n"
            )
            second = root / "crates" / "apps" / "example" / "src" / "flume.rs"
            second.write_text(
                "fn chans() { let (_tx, _rx) = flume::unbounded(); }\n"
            )
            violations = panic_and_lint.scan_unbounded_channels()
            self.assertEqual(len(violations), 2, violations)
