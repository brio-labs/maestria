"""Tests for the philosophy_check secrets family."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from philosophy_check import secrets, shared
from philosophy_check_testbase import PhilosophyCheckFixture


class SecretsTests(PhilosophyCheckFixture):

    def test_hardcoded_secrets_reports_private_key_and_token(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "docs" / "deploy.md"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "-----BEGIN RSA PRIVATE KEY-----\n" "export AWS_KEY=AKIA1234567890ABCDEF\n"
            )
            violations = secrets.scan_hardcoded_secrets()
            self.assertTrue(any("private key material" in item for item in violations), violations)
            self.assertTrue(any("access-token pattern" in item for item in violations), violations)

    def test_hardcoded_secrets_reports_credential_assignment_in_code(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "scripts" / "deploy.py"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text('password = "hunter2"\n')
            violations = secrets.scan_hardcoded_secrets()
            self.assertTrue(any("credential assignment" in item for item in violations), violations)

    def test_hardcoded_secrets_skips_tests_and_ci_templates(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            crate_root = root / "crates" / "apps" / "example"
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["crates/apps/example"]\n',
                encoding="utf-8",
            )
            source_dir = crate_root / "src"
            source_dir.mkdir(parents=True)
            (crate_root / "Cargo.toml").write_text(
                '[package]\nname = "example"\n'
                'version = "0.1.0"\nedition = "2021"\n',
                encoding="utf-8",
            )
            (source_dir / "lib.rs").write_text("", encoding="utf-8")
            test_file = crate_root / "tests" / "fixture.rs"
            test_file.parent.mkdir(parents=True)
            test_file.write_text(
                'const FAKE: &str = "AKIA1234567890ABCDEF";\n',
                encoding="utf-8",
            )
            workflow = root / ".github" / "workflows" / "ci.yml"
            workflow.parent.mkdir(parents=True)
            workflow.write_text(
                "password: ${{ secrets.DB_PASSWORD }}\n"
                "token: <contents of system/daemon.token>\n",
                encoding="utf-8",
            )
            violations = secrets.scan_hardcoded_secrets()
            self.assertEqual(violations, [])

    def test_hardcoded_secrets_skips_inline_cfg_test_fixtures(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "kernel" / "sillage-domain" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "#[cfg(test)]\n" "mod tests { const FAKE: &str = \"-----BEGIN PRIVATE KEY-----\"; }\n"
            )
            violations = secrets.scan_hardcoded_secrets()
            self.assertEqual(violations, [])

    def test_hardcoded_secrets_accepts_struct_fields_and_expressions(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.configure_root(root)
            source = root / "crates" / "apps" / "example" / "src" / "lib.rs"
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(
                "struct Request { token: String }\n"
                "fn clone(token: &str) { let _ = token; }\n"
                "fn assign(token: String) { let copy = token; }\n"
            )
            violations = secrets.scan_hardcoded_secrets()
            self.assertEqual(violations, [])