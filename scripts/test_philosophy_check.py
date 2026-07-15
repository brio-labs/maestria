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

    def test_scan_markers_reports_task_marker(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "maestria-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True)
            source.write_text("// " + "TO" + "DO" + ": remove marker\n", encoding="utf-8")

            self.assertEqual(PHILOSOPHY_CHECK.scan_markers(), ["crates/kernel/maestria-domain/src/lib.rs"])

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



if __name__ == "__main__":
    unittest.main()
