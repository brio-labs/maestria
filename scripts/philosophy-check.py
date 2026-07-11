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
MAX_PRODUCTION_LOGICAL_LINES = 400
MAX_MODULE_PHYSICAL_LINES = 900
MODULE_SIZE_EXEMPTIONS = {
    "crates/kernel/maestria-domain/src/types.rs": (
        "canonical domain type catalog; split requires a coordinated domain API migration"
    ),
    "crates/kernel/maestria-domain/src/replay.rs": (
        "single replay reducer; split requires preserving event ordering and replay proofs"
    ),
    "crates/kernel/maestria-domain/src/input/handlers.rs": (
        "existing transition handler catalog; split requires coordinated domain transition proofs"
    ),
    "crates/kernel/maestria-domain/tests/replay_integration.rs": (
        "existing replay integration suite; split is tracked with the domain test migration"
    ),
    "crates/kernel/maestria-domain/tests/domain_unit_tests.rs": (
        "existing domain behavior suite; split is tracked with the domain test migration"
    ),
    "crates/kernel/maestria-ports/src/in_memory.rs": (
        "existing in-memory adapter catalog; split requires shared contract coordination"
    ),
    "crates/kernel/maestria-ports/src/contract_tests.rs": (
        "existing adapter contract suite; split requires preserving shared contract fixtures"
    ),
    "crates/runtime/maestria-runtime/src/lib.rs": (
        "existing runtime effect loop; split requires coordinated worker lifecycle changes"
    ),
    "crates/apps/maestria-cli/src/main.rs": (
        "existing CLI command catalog; split requires coordinated command registration changes"
    ),
    "crates/storage/maestria-storage-sqlite/src/tests.rs": (
        "existing storage compatibility suite; split requires preserving migration fixtures"
    ),
    "crates/storage/maestria-storage-sqlite/src/repositories.rs": (
        "existing storage repository catalog; split requires coordinated DTO boundary changes"
    ),
    "crates/storage/maestria-storage-sqlite/src/event_payloads.rs": (
        "existing event DTO catalog; split requires preserving replay serialization compatibility"
    ),
    "crates/kernel/maestria-governance/src/lib.rs": (
        "existing governance policy catalog; split requires coordinated public policy API changes"
    ),
    "crates/storage/maestria-vector-sqlite/src/lib.rs": (
        "existing vector projection catalog; split requires preserving adapter migration behavior"
    ),
    "crates/storage/maestria-graph-sqlite/src/lib.rs": (
        "existing graph projection catalog; split requires preserving adapter migration behavior"
    ),
    "crates/ecosystem/maestria-parsers/src/lib.rs": (
        "existing parser registry catalog; split requires coordinated parser contract changes"
    ),
    "crates/ecosystem/maestria-validation/src/lib.rs": (
        "existing validator catalog; split requires preserving validator registration behavior"
    ),
}


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


def logical_line_count(content: str) -> int:
    return sum(
        1
        for line in content.splitlines()
        if line.strip() and not line.lstrip().startswith("//")
    )


def scan_module_sizes() -> list[str]:
    violations = []
    for source in ROOT.rglob("*.rs"):
        if should_skip(source):
            continue
        rel_path = source.relative_to(ROOT)
        rel = rel_path.as_posix()
        if rel in MODULE_SIZE_EXEMPTIONS:
            continue
        content = read_text(source)
        if content is None:
            continue
        is_test_file = "tests" in rel_path.parts
        logical_lines = logical_line_count(content)
        physical_lines = len(content.splitlines())
        if not is_test_file and logical_lines > MAX_PRODUCTION_LOGICAL_LINES:
            violations.append(
                f"{rel} has {logical_lines} module logical lines "
                f"(limit {MAX_PRODUCTION_LOGICAL_LINES})"
            )
        if physical_lines > MAX_MODULE_PHYSICAL_LINES:
            violations.append(
                f"{rel} has {physical_lines} physical lines "
                f"(limit {MAX_MODULE_PHYSICAL_LINES})"
            )
    return violations


def main() -> int:
    violations = []
    marker_violations = scan_markers()
    violations.extend(f"{path} contains forbidden task marker" for path in marker_violations)
    violations.extend(scan_domain_manifest())
    violations.extend(scan_domain_sources())
    violations.extend(scan_module_sizes())

    if violations:
        print("philosophy-check failed:")
        for violation in violations:
            print(f" - {violation}")
        return 1

    print("philosophy-check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
