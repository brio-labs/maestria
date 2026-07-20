from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("codeowners-check.py")
SPEC = importlib.util.spec_from_file_location("codeowners_check", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load codeowners-check.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class CodeownersCheckTests(unittest.TestCase):
    """Focused unit tests for the CODEOWNERS invariant check."""

    def setUp(self) -> None:
        self._old_root = CHECK.ROOT
        self._old_required = CHECK.REQUIRED_OWNERSHIP_PATHS

    def tearDown(self) -> None:
        CHECK.ROOT = self._old_root
        CHECK.REQUIRED_OWNERSHIP_PATHS = self._old_required

    # ── helpers ──────────────────────────────────────────────────────

    def make_owned(self, root: Path, *paths: str) -> set[str]:
        owned: set[str] = set()
        for p in paths:
            owned.add(p)
            target = root / p.lstrip("/").rstrip("/")
            if p.endswith("/"):
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("", encoding="utf-8")
        return owned

    def make_codeowners(self, root: Path, lines: list[str]) -> Path:
        target = root / ".github" / "CODEOWNERS"
        target.parent.mkdir(parents=True)
        target.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return target

    # ── parse_codeowners ─────────────────────────────────────────────

    def test_parse_codeowners_empty(self) -> None:
        self.assertEqual(CHECK.parse_codeowners([]), [])

    def test_parse_codeowners_skips_comments_and_blanks(self) -> None:
        lines = [
            "# This is a comment",
            "",
            "  ",
            "/path/ @owner",
            "",
        ]
        self.assertEqual(CHECK.parse_codeowners(lines), [("/path/", "@owner")])

    def test_parse_codeowners_multiple_rules(self) -> None:
        lines = [
            "/crates/kernel/  @team-a",
            "/docs/          @team-b",
            "*.md             @team-c",
        ]
        self.assertEqual(
            CHECK.parse_codeowners(lines),
            [
                ("/crates/kernel/", "@team-a"),
                ("/docs/", "@team-b"),
                ("*.md", "@team-c"),
            ],
        )

    # ── check_required_coverage ──────────────────────────────────────

    def test_all_required_present(self) -> None:
        CHECK.REQUIRED_OWNERSHIP_PATHS = {"/required/path/", "/other/file"}
        owned = {"/required/path/", "/other/file", "/extra/"}
        self.assertEqual(CHECK.check_required_coverage(owned), [])

    def test_missing_required_reported(self) -> None:
        CHECK.REQUIRED_OWNERSHIP_PATHS = {"/required/path/", "/missing/"}
        owned = {"/required/path/"}
        self.assertEqual(CHECK.check_required_coverage(owned), ["/missing/"])

    # ── all_entries_exist ────────────────────────────────────────────

    def test_all_entries_exist(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            CHECK.ROOT = root
            owned = self.make_owned(root, "/dir/", "/file.txt")
            self.assertEqual(CHECK.all_entries_exist(owned), [])

    def test_missing_entry_reported(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            CHECK.ROOT = root
            owned = self.make_owned(root, "/dir/")
            owned |= {"/missing-dir/", "/missing-file.py"}
            self.assertEqual(
                sorted(CHECK.all_entries_exist(owned)),
                ["/missing-dir/", "/missing-file.py"],
            )

    def test_glob_pattern_skipped(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            CHECK.ROOT = root
            owned = {"*", "*.md", "/crates/*/src/"}
            self.assertEqual(CHECK.all_entries_exist(owned), [])

    # ── check_invalid_rules ──────────────────────────────────────────

    def test_valid_rules_pass(self) -> None:
        lines = [
            "/path/ @owner",
            "*.md @team",
        ]
        self.assertEqual(CHECK.check_invalid_rules(lines), [])

    def test_malformed_line_detected(self) -> None:
        lines = [
            "# comment",
            "/path/ @owner",
            "no-owner-here",
            "  ",
        ]
        errors = CHECK.check_invalid_rules(lines)
        self.assertEqual(len(errors), 1)
        self.assertIn("no-owner-here", errors[0])

    def test_comments_and_blanks_ignored(self) -> None:
        lines = [
            "# comment",
            "",
            "  ",
            "/path/ @owner",
        ]
        self.assertEqual(CHECK.check_invalid_rules(lines), [])

    # ── integration: main() ──────────────────────────────────────────

    def test_main_passes_when_valid(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            CHECK.ROOT = root
            CHECK.CODEOWNERS_PATH = self.make_codeowners(root, [
                "/crates/kernel/maestria-domain/   @carabistouflette",
                "/crates/kernel/maestria-governance/ @carabistouflette",
                "/crates/kernel/maestria-ports/      @carabistouflette",
                "/crates/runtime/                    @carabistouflette",
                "/crates/ecosystem/maestria-retrieval/ @carabistouflette",
                "/crates/ecosystem/maestria-validation/ @carabistouflette",
                "/crates/storage/                    @carabistouflette",
                "/crates/harness/                    @carabistouflette",
                "/crates/apps/maestria-daemon/       @carabistouflette",
                "/tests/property/                    @carabistouflette",
                "/tests/replay/                      @carabistouflette",
                "/docs/PHILOSOPHY.md                 @carabistouflette",
                "/docs/SECURITY.md                   @carabistouflette",
                "/scripts/philosophy-check.py        @carabistouflette",
                "/scripts/release-contract.sh        @carabistouflette",
                "/scripts/release_exit_evidence.py   @carabistouflette",
                "/scripts/version.py                 @carabistouflette",
                "/scripts/strict-clippy.sh           @carabistouflette",
                ".github/workflows/release.yml       @carabistouflette",
                ".github/workflows/ci.yml            @carabistouflette",
                "/deny.toml                          @carabistouflette",
            ])
            # Create all required paths as files/dirs so they exist.
            for p in CHECK.REQUIRED_OWNERSHIP_PATHS:
                target = root / p.lstrip("/").rstrip("/")
                if p.endswith("/"):
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("", encoding="utf-8")
            self.assertEqual(CHECK.main(), 0)

    def test_main_fails_when_paths_missing(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            CHECK.ROOT = root
            CHECK.CODEOWNERS_PATH = self.make_codeowners(root, [
                "* @default",
            ])
            self.assertEqual(CHECK.main(), 1)

    def test_main_fails_when_harness_missing(self) -> None:
        """Verify that removing a required path (harness) triggers failure."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            CHECK.ROOT = root
            CHECK.CODEOWNERS_PATH = self.make_codeowners(root, [
                "/crates/kernel/maestria-domain/   @carabistouflette",
                "/crates/kernel/maestria-governance/ @carabistouflette",
                "/crates/kernel/maestria-ports/      @carabistouflette",
                "/crates/runtime/                    @carabistouflette",
                "/crates/ecosystem/maestria-retrieval/ @carabistouflette",
                "/crates/ecosystem/maestria-validation/ @carabistouflette",
                "/crates/storage/                    @carabistouflette",
                # /crates/harness/ intentionally omitted
                "/crates/apps/maestria-daemon/       @carabistouflette",
                "/tests/property/                    @carabistouflette",
                "/tests/replay/                      @carabistouflette",
                "/docs/PHILOSOPHY.md                 @carabistouflette",
                "/docs/SECURITY.md                   @carabistouflette",
                "/scripts/philosophy-check.py        @carabistouflette",
                "/scripts/release-contract.sh        @carabistouflette",
                "/scripts/release_exit_evidence.py   @carabistouflette",
                "/scripts/version.py                 @carabistouflette",
                "/scripts/strict-clippy.sh           @carabistouflette",
                ".github/workflows/release.yml       @carabistouflette",
                ".github/workflows/ci.yml            @carabistouflette",
                "/deny.toml                          @carabistouflette",
            ])
            for p in CHECK.REQUIRED_OWNERSHIP_PATHS:
                target = root / p.lstrip("/").rstrip("/")
                if p.endswith("/"):
                    target.mkdir(parents=True, exist_ok=True)
                else:
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("", encoding="utf-8")
            self.assertEqual(CHECK.main(), 1)


if __name__ == "__main__":
    unittest.main()
