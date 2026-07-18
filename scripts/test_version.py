from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("version.py")
SPEC = importlib.util.spec_from_file_location("version", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load version.py")
VERSION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VERSION)


class VersionContractTests(unittest.TestCase):
    def write_repository(self, root: Path, *, package_version: str = "version.workspace = true") -> None:
        (root / "crates" / "example").mkdir(parents=True)
        (root / "Cargo.toml").write_text(
            "[package]\nname = \"maestria\"\n"
            f"{package_version}\n"
            "edition.workspace = true\nlicense.workspace = true\nrust-version.workspace = true\n\n"
            "[workspace]\nmembers = [\"crates/example\"]\n\n"
            "[workspace.package]\nversion = \"0.6.1\"\nedition = \"2024\"\n"
            "license = \"MIT OR Apache-2.0\"\nrust-version = \"1.95\"\n",
            encoding="utf-8",
        )
        (root / "crates" / "example" / "Cargo.toml").write_text(
            "[package]\nname = \"example\"\nversion.workspace = true\n"
            "edition.workspace = true\nlicense.workspace = true\nrust-version.workspace = true\n",
            encoding="utf-8",
        )
        (root / "Cargo.lock").write_text(
            "version = 4\n\n"
            "[[package]]\nname = \"maestria\"\nversion = \"0.6.1\"\n\n"
            "[[package]]\nname = \"example\"\nversion = \"0.6.1\"\n",
            encoding="utf-8",
        )

    def test_check_accepts_one_canonical_version(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_repository(root)
            self.assertEqual(VERSION.check(root), [])
            self.assertEqual(VERSION.canonical_version(root), "0.6.1")

    def test_check_rejects_manifest_version_literal(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_repository(root, package_version='version = "0.6.1"')
            self.assertEqual(
                VERSION.check(root),
                ["Cargo.toml must inherit version.workspace = true"],
            )

    def test_check_rejects_lockfile_drift(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_repository(root)
            lock = root / "Cargo.lock"
            lock.write_text(lock.read_text(encoding="utf-8").replace("0.6.1", "0.6.0"), encoding="utf-8")
            self.assertEqual(
                VERSION.check(root),
                [
                    "Cargo.lock does not record maestria at 0.6.1 (Cargo.toml)",
                    "Cargo.lock does not record example at 0.6.1 (crates/example/Cargo.toml)",
                ],
            )

    def test_replace_workspace_version_changes_only_workspace_section(self) -> None:
        text = "[package]\nversion.workspace = true\n\n[workspace.package]\nversion = \"0.6.1\"\n\n[dependencies]\nfoo = \"0.6.1\"\n"
        updated = VERSION.replace_workspace_version(text, "0.6.2")
        self.assertIn('[workspace.package]\nversion = "0.6.2"', updated)
        self.assertIn('foo = "0.6.1"', updated)

    def test_update_lock_versions_changes_only_workspace_packages(self) -> None:
        lock = (
            "version = 4\n\n"
            "[[package]]\nname = \"maestria\"\nversion = \"0.6.1\"\n\n"
            "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n"
        )
        updated = VERSION.update_lock_versions(lock, {"maestria"}, "0.6.2")
        self.assertIn('name = "maestria"\nversion = "0.6.2"', updated)
        self.assertIn('name = "serde"\nversion = "1.0.0"', updated)

    def test_semver_validation_rejects_release_prefix(self) -> None:
        self.assertIsNone(VERSION.SEMVER.fullmatch("v0.6.2"))
        self.assertIsNotNone(VERSION.SEMVER.fullmatch("0.6.2"))
        self.assertIsNotNone(VERSION.SEMVER.fullmatch("1.0.0-rc.1+build.7"))


if __name__ == "__main__":
    unittest.main()
