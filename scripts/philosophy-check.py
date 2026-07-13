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
KERNEL_ROOTS = (
    ROOT / "crates" / "kernel" / "maestria-domain",
    ROOT / "crates" / "kernel" / "maestria-governance",
    ROOT / "crates" / "kernel" / "maestria-ports",
)
SCAN_EXTS = {".rs", ".toml", ".py", ".yml", ".yaml", ".md"}
SKIP_DIRS = {".git", "target", "node_modules", "dist", ".direnv", ".venv"}
SKIP_FILES = {"maestria_brioche_informed_code_architecture_report.md"}
FORBIDDEN_MARKERS = [r"\bTODO\b", r"\bFIXME\b"]
FORBIDDEN_KERNEL_DEPENDENCIES = {
    "tokio",
    "sqlx",
    "reqwest",
    "tantivy",
    "axum",
    "hyper",
    "tonic",
    "actix-web",
}
FORBIDDEN_KERNEL_TOKENS = [
    "std::fs",
    "std::process",
    "SystemTime",
    "Instant::now",
]
FORBIDDEN_DOMAIN_FAILURES = [
    "unwrap(",
    "expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
]
MAX_PRODUCTION_LOGICAL_LINES = 400
MAX_MODULE_PHYSICAL_LINES = 900
MODULE_SIZE_EXEMPTIONS: dict[str, str] = {}
KERNEL_ALLOWED_DEPENDENCIES = {
    "maestria-domain": {"sha2"},
    "maestria-governance": {"maestria_domain"},
    "maestria-ports": {"maestria_domain"},
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


def _manifest_dependencies(content: str) -> set[str]:
    try:
        import tomllib

        document = tomllib.loads(content)
    except (tomllib.TOMLDecodeError, ValueError):
        return set()
    dependencies = set(document.get("dependencies", {}))
    for target in document.get("target", {}).values():
        dependencies.update(target.get("dependencies", {}))
    return dependencies


def scan_kernel_manifests() -> list[str]:
    violations = []
    for kernel_root in KERNEL_ROOTS:
        manifest = kernel_root / "Cargo.toml"
        content = read_text(manifest)
        if content is None:
            violations.append(str(manifest.relative_to(ROOT)))
            continue
        dependencies = _manifest_dependencies(content)
        crate_name = kernel_root.name
        allowed = KERNEL_ALLOWED_DEPENDENCIES.get(crate_name, set())
        for dependency in sorted(dependencies & FORBIDDEN_KERNEL_DEPENDENCIES):
            violations.append(
                f"{manifest.relative_to(ROOT)} contains forbidden dependency token {dependency}"
            )
        for dependency in sorted(
            dependency
            for dependency in dependencies
            if dependency.startswith("maestria_") and dependency not in allowed
        ):
            violations.append(
                f"{manifest.relative_to(ROOT)} contains disallowed kernel dependency {dependency}"
            )
    return violations


def is_test_source(path: Path) -> bool:
    return (
        "tests" in path.parts
        or path.stem in {"tests", "contract_tests"}
        or path.stem.endswith("_tests")
    )


def scan_kernel_sources() -> list[str]:
    violations = []
    for kernel_root in KERNEL_ROOTS:
        for source in (kernel_root / "src").rglob("*.rs"):
            if is_test_source(source):
                continue
            content = read_text(source)
            if content is None:
                continue
            production = production_rust(content)
            rel = source.relative_to(ROOT)
            for token in FORBIDDEN_KERNEL_TOKENS:
                if token in production:
                    violations.append(f"{rel} contains forbidden kernel token {token}")
            for token in FORBIDDEN_DOMAIN_FAILURES:
                if token in production:
                    violations.append(f"{rel} contains forbidden failure token {token}")
    return violations


def scan_domain_manifest() -> list[str]:
    content = read_text(DOMAIN_MANIFEST)
    if content is None:
        return [str(DOMAIN_MANIFEST.relative_to(ROOT))]
    violations = []
    for dependency in sorted(
        _manifest_dependencies(content) & FORBIDDEN_KERNEL_DEPENDENCIES
    ):
        violations.append(
            f"{DOMAIN_MANIFEST.relative_to(ROOT)} contains forbidden dependency token {dependency}"
        )
    return violations


def scan_domain_sources() -> list[str]:
    violations = []
    for source in DOMAIN_SRC.rglob("*.rs"):
        content = read_text(source)
        if content is None:
            continue
        production = production_rust(content)
        rel = source.relative_to(ROOT)
        for token in FORBIDDEN_KERNEL_TOKENS:
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
        content = read_text(source)
        if content is None:
            continue
        logical_lines = logical_line_count(content)
        physical_lines = len(content.splitlines())
        if logical_lines > MAX_PRODUCTION_LOGICAL_LINES and not is_test_source(rel_path):
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
    violations.extend(scan_kernel_manifests())
    violations.extend(scan_kernel_sources())
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
