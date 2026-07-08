#!/usr/bin/env python3
"""Repository doctrine checks for the Maestria bootstrap."""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
THIS_SCRIPT = Path(__file__).resolve()
DOMAIN_ROOT = ROOT / "crates" / "kernel" / "maestria-domain"
DOMAIN_SRC = DOMAIN_ROOT / "src"
DOMAIN_MANIFEST = DOMAIN_ROOT / "Cargo.toml"
SCAN_EXTS = {".rs", ".toml", ".py", ".yml", ".yaml", ".md"}
SKIP_DIRS = {".git", "target", "node_modules", "dist", ".direnv", ".venv"}
SKIP_FILES = {"maestria_brioche_informed_code_architecture_report.md"}
FORBIDDEN_MARKERS = [r"\bTODO\b", r"\bFIXME\b"]
FORBIDDEN_DOMAIN_TOKENS = [
    "tokio",
    "sqlx",
    "reqwest",
    "tantivy",
    "axum",
    "std::fs",
    "std::process",
    "SystemTime",
    "Instant::now",
]
FORBIDDEN_DOMAIN_FAILURES = ["unwrap(", "expect(", "panic!("]


def should_skip(path: Path) -> bool:
    rel = path.relative_to(ROOT)
    rel_parts = set(rel.parts)
    return (
        path.resolve() == THIS_SCRIPT
        or rel.as_posix() in SKIP_FILES
        or bool(rel_parts.intersection(SKIP_DIRS))
    )


def read_text(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


def production_rust(text: str) -> str:
    return text.split("#[cfg(test)]", 1)[0]


def scan_markers() -> list[str]:
    violations = []
    for candidate in ROOT.rglob("*"):
        if candidate.is_dir() or should_skip(candidate):
            continue
        if candidate.suffix.lower() not in SCAN_EXTS:
            continue
        content = read_text(candidate)
        if content is None:
            continue
        if any(re.search(pattern, content) for pattern in FORBIDDEN_MARKERS):
            violations.append(str(candidate.relative_to(ROOT)))
    return violations


def scan_domain_manifest() -> list[str]:
    content = read_text(DOMAIN_MANIFEST)
    if content is None:
        return [str(DOMAIN_MANIFEST.relative_to(ROOT))]
    return [
        f"{DOMAIN_MANIFEST.relative_to(ROOT)} contains forbidden dependency token {token}"
        for token in FORBIDDEN_DOMAIN_TOKENS
        if f"{token} =" in content
    ]


def scan_domain_sources() -> list[str]:
    violations = []
    for source in DOMAIN_SRC.rglob("*.rs"):
        content = read_text(source)
        if content is None:
            continue
        production = production_rust(content)
        rel = source.relative_to(ROOT)
        for token in FORBIDDEN_DOMAIN_TOKENS:
            if token in production:
                violations.append(f"{rel} contains forbidden domain token {token}")
        for token in FORBIDDEN_DOMAIN_FAILURES:
            if token in production:
                violations.append(f"{rel} contains forbidden failure token {token}")
    return violations


def main() -> int:
    violations = []
    marker_violations = scan_markers()
    violations.extend(f"{path} contains forbidden task marker" for path in marker_violations)
    violations.extend(scan_domain_manifest())
    violations.extend(scan_domain_sources())

    if violations:
        print("philosophy-check failed:")
        for violation in violations:
            print(f" - {violation}")
        return 1

    print("philosophy-check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
