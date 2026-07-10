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
        }

    def tearDown(self) -> None:
        for name, value in self._old_globals.items():
            setattr(PHILOSOPHY_CHECK, name, value)

    def configure_root(self, root: Path) -> None:
        domain_root = root / "crates" / "kernel" / "maestria-domain"
        setattr(PHILOSOPHY_CHECK, "ROOT", root)
        setattr(PHILOSOPHY_CHECK, "THIS_SCRIPT", root / "scripts" / "philosophy-check.py")
        setattr(PHILOSOPHY_CHECK, "DOMAIN_ROOT", domain_root)
        setattr(PHILOSOPHY_CHECK, "DOMAIN_SRC", domain_root / "src")
        setattr(PHILOSOPHY_CHECK, "DOMAIN_MANIFEST", domain_root / "Cargo.toml")

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



    def test_rust_size_limits_are_enforced(self) -> None:
        self.assertEqual(
            PHILOSOPHY_CHECK.rust_size_violations("src/domain.rs", 1201, 1001, False),
            [
                "src/domain.rs has 1201 Rust lines; split the module (maximum 1200)",
                "src/domain.rs has 1001 production Rust lines; split the module by responsibility (maximum 1000)",
            ],
        )
        self.assertEqual(
            PHILOSOPHY_CHECK.rust_size_violations("src/tests.rs", 1201, 1201, True),
            ["src/tests.rs has 1201 Rust lines; split the module (maximum 1200)"],
        )

if __name__ == "__main__":
    unittest.main()
