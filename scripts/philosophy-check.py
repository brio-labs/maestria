#!/usr/bin/env python3
"""Lightweight architecture guardrails for bootstrap.

Current checks are intentionally minimal and conservative so the project can
bootstrap cleanly while still catching obvious drift.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
THIS_SCRIPT = Path(__file__).resolve()
SCAN_EXTS = {".rs", ".toml", ".py", ".yml", ".yaml"}
SKIP_DIRS = {".git", "target", "node_modules", "dist", ".direnv", ".venv"}
PATTERNS = [r"\bTODO\b", r"\bFIXME\b"]


def should_skip(path: Path) -> bool:
    rel_parts = set(path.relative_to(ROOT).parts)
    return path.resolve() == THIS_SCRIPT or bool(rel_parts.intersection(SKIP_DIRS))


def has_bad_comment(text: str) -> bool:
    return any(re.search(pattern, text) for pattern in PATTERNS)


def main() -> int:
    violations = []
    for candidate in ROOT.rglob("*"):
        if candidate.is_dir() or should_skip(candidate):
            continue
        if candidate.suffix.lower() not in SCAN_EXTS:
            continue
        try:
            content = candidate.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        if has_bad_comment(content):
            violations.append(str(candidate.relative_to(ROOT)))

    if violations:
        print("philosophy-check failed: unexpected TODO/FIXME markers:")
        for path in violations:
            print(f" - {path}")
        return 1

    print("philosophy-check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
