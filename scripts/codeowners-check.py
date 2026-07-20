#!/usr/bin/env python3
"""Deterministic CODEOWNERS invariant check for the Maestria repository.

Verifies:
  1. Every required invariant-owning path has an explicit CODEOWNERS entry.
  2. Every non-glob path in CODEOWNERS exists in the repository.
  3. The CODEOWNERS file is syntactically valid.

Run directly (no extra dependencies) or import as a module.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CODEOWNERS_PATH = ROOT / ".github" / "CODEOWNERS"
THIS_SCRIPT = Path(__file__).resolve()

# The paths that MUST have explicit CODEOWNERS entries because they own
# architectural invariants.  These align with the layers from
# docs/ARCHITECTURE.md and the invariant matrix in docs/invariants/matrix.md.
# Adding or removing an invariant-owning path requires updating this list
# AND the acceptance criteria in the corresponding test.
REQUIRED_OWNERSHIP_PATHS: set[str] = {
    # ── Core kernel ────────────────────────────────────────────
    "/crates/kernel/maestria-domain/",
    "/crates/kernel/maestria-governance/",
    "/crates/kernel/maestria-ports/",
    # ── Runtime ────────────────────────────────────────────────
    "/crates/runtime/",
    # ── Retrieval ──────────────────────────────────────────────
    "/crates/ecosystem/maestria-retrieval/",
    # ── Validation ─────────────────────────────────────────────
    "/crates/ecosystem/maestria-validation/",
    # ── Storage ────────────────────────────────────────────────
    "/crates/storage/",
    # ── Harness ────────────────────────────────────────────────
    "/crates/harness/",
    # ── Daemon ─────────────────────────────────────────────────
    "/crates/apps/maestria-daemon/",
    # ── Test suites ────────────────────────────────────────────
    "/tests/property/",
    "/tests/replay/",
    # ── Philosophy ─────────────────────────────────────────────
    "/docs/PHILOSOPHY.md",
    "/scripts/philosophy-check.py",
    # ── Release workflow ───────────────────────────────────────
    "/scripts/release-contract.sh",
    "/scripts/release_exit_evidence.py",
    "/scripts/version.py",
    ".github/workflows/release.yml",
    # ── Security ───────────────────────────────────────────────
    "/deny.toml",
    "/scripts/strict-clippy.sh",
    "/docs/SECURITY.md",
    ".github/workflows/ci.yml",
}

# Lines that look like a path/owner rule (ignoring leading whitespace and comments).
RULE_LINE = re.compile(r"^\s*(?P<path>\S+)\s+(?P<owner>@\S+|[\w-]+/[\w-]+)\s*(?:#.*)?$")


def read_lines(path: Path) -> list[str]:
    """Return non-empty lines stripped of trailing whitespace."""
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return []
    return text.splitlines()


def parse_codeowners(lines: list[str]) -> list[tuple[str, str]]:
    """Parse CODEOWNERS into a list of (path_pattern, owner) tuples.

    Comments and blank lines are ignored.  Returns tuples in file order.
    """
    rules: list[tuple[str, str]] = []
    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        m = RULE_LINE.match(stripped)
        if m:
            rules.append((m.group("path"), m.group("owner")))
    return rules


def get_owned_paths(rules: list[tuple[str, str]]) -> set[str]:
    """Return the set of path patterns that have explicit owners."""
    return {path for path, _ in rules}


def _is_glob_pattern(pattern: str) -> bool:
    """Return True if the pattern contains glob characters (*, ?, [,])."""
    return bool(set("*?[") & set(pattern))


def all_entries_exist(owned: set[str]) -> list[str]:
    """Return paths in *owned* whose target does not exist in the repository.

    Glob patterns (containing ``*``, ``?``, ``[...]``) are skipped because they
    describe classes of paths rather than single filesystem entries.
    """
    missing: list[str] = []
    for path_str in sorted(owned):
        if _is_glob_pattern(path_str):
            continue
        clean = path_str.lstrip("/").rstrip("/")
        candidate = ROOT / clean
        if path_str.endswith("/"):
            if not candidate.is_dir():
                missing.append(path_str)
        else:
            if not candidate.is_file():
                missing.append(path_str)
    return missing


def check_required_coverage(owned: set[str]) -> list[str]:
    """Return required paths missing from the CODEOWNERS rule set."""
    return sorted(REQUIRED_OWNERSHIP_PATHS - owned)


def get_codeowners_lines() -> list[str]:
    """Read CODEOWNERS lines. Returns empty list if file is missing."""
    return read_lines(CODEOWNERS_PATH)


def check_invalid_rules(lines: list[str]) -> list[str]:
    """Return malformed non-comment lines that are not valid rules."""
    errors: list[str] = []
    for i, line in enumerate(lines, 1):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if not RULE_LINE.match(stripped):
            errors.append(f"  line {i}: {stripped!r} does not match <path> <owner>")
    return errors


def main() -> int:
    """Run all checks and print violations.  Returns 0 on success, 1 on failure."""
    violations: list[str] = []

    lines = get_codeowners_lines()
    if not lines:
        print("❌ CODEOWNERS file not found or empty")
        return 1

    invalid = check_invalid_rules(lines)
    if invalid:
        violations.append("Invalid CODEOWNERS rules:")
        violations.extend(invalid)

    rules = parse_codeowners(lines)
    owned = get_owned_paths(rules)

    missing_required = check_required_coverage(owned)
    if missing_required:
        violations.append("Required invariant-owning paths missing CODEOWNERS entries:")
        for p in missing_required:
            violations.append(f"  {p}")

    nonexistent = all_entries_exist(owned)
    if nonexistent:
        violations.append("CODEOWNERS entries refer to non-existent paths:")
        for p in nonexistent:
            violations.append(f"  {p}")

    if violations:
        print("❌ CODEOWNERS invariant check failed:")
        for v in violations:
            print(v)
        return 1

    print("✓ CODEOWNERS invariant check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
