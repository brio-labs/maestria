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
FORBIDDEN_RUST_LINT_BYPASSES = [
    r"#\s*!?\s*\[\s*allow\b",
    r"#\s*!?\s*\[\s*cfg_attr\s*\([^]]*\ballow\b",
]
FORBIDDEN_RUST_METHODS = [
    (
        r"\.(?:unwrap|expect|unwrap_err|expect_err|unwrap_or|unwrap_or_else|unwrap_or_default)\s*\(",
        "a forbidden Option/Result failure method",
    ),
    (r"\b(?:HashMap|HashSet)::new\s*\(", "a forbidden hash collection constructor"),
    (r"\bstd::time::Instant::now\s*\(", "a forbidden wall-clock instant"),
    (r"\.swap_remove\s*\(", "a forbidden swap_remove call"),
]
MAX_PRODUCTION_LOGICAL_LINES = 400
MAX_MODULE_PHYSICAL_LINES = 900
MODULE_SIZE_EXEMPTIONS: dict[str, str] = {}
KERNEL_ALLOWED_DEPENDENCIES = {
    "maestria-domain": {"sha2"},
    "maestria-governance": {"maestria_domain"},
    "maestria-ports": {"maestria_domain"},
}

CANONICAL_DOC_MARKERS = {
    "docs/ARCHITECTURE.md": ("authoritative state", "external factual truth"),
    "docs/SEARCH.md": ("SearchPlan", "SearchTraceId", "abstention"),
    "docs/MEMORY.md": ("MemoryCandidate", "provenance", "staleness"),
    "docs/SECURITY.md": ("prompt injection", "quarantine", "before scoring"),
    "docs/OPERATIONS.md": ("bounded", "recovery", "projection"),
    "docs/ROADMAP.md": ("single canonical", "exit criteria"),
    "docs/RESEARCH.md": ("NON-NORMATIVE", "quality", "security", "energy"),
}
CANONICAL_DOC_SECTIONS = {
    "docs/ARCHITECTURE.md": ("## 2. System Identity", "## 3. Architectural Dependency Direction"),
    "docs/SEARCH.md": ("## Search Boundary Objects", "## Search Execution Model", "## Budgets and Stop Conditions"),
    "docs/MEMORY.md": ("## 1. Information Lifecycle", "## 2. Provenance and Staleness", "## 3. Boundaries and Overclaiming"),
    "docs/SECURITY.md": ("## 2. Security Invariants", "## 5. Taint and Quarantine", "## 6. Prompt Injection as Data"),
    "docs/OPERATIONS.md": ("## 1. Bounded Runtime Lifecycle", "## 2. State and Recovery", "## 4. Data Evolution"),
    "docs/ROADMAP.md": ("## Phase 1:", "## Phase 6:"),
    "docs/RESEARCH.md": ("## 1. Evaluation Framework", "## 3. Promotion Criteria"),
}
POLICY_DOC_MARKERS = {
    "docs/PHILOSOPHY.md": (
        "41. Search plans",
        "42. Search traces",
        "43. Every retrieval lane",
        "44. Retrieval changes",
        "45. Normative architecture",
        "46. Maestria preserves",
        "47. Model-generated search plans",
    ),
    "docs/SPECS.md": (
        "I-Search-TypedBudgeted",
        "I-Search-TraceFingerprint",
        "I-Search-SecurityBeforeScore",
        "I-Search-Evaluated",
    ),
}
FORBIDDEN_EXTERNAL_TRUTH_WORDING = (
    "domain owns truth",
    "truth machine",
    "truth store",
    "truth owner",
)


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

def scan_documentation_contract() -> list[str]:
    violations = []
    for relative_path, markers in CANONICAL_DOC_MARKERS.items():
        path = ROOT / relative_path
        content = read_text(path)
        if content is None:
            violations.append(f"{relative_path} is missing or unreadable")
            continue
        lowered = content.casefold()
        for marker in markers:
            if marker.casefold() not in lowered:
                violations.append(f"{relative_path} is missing required marker {marker!r}")
        lines = {line.strip() for line in content.splitlines()}
        for section in CANONICAL_DOC_SECTIONS[relative_path]:
            section_found = (
                any(line.startswith(section) for line in lines)
                if section.endswith(":")
                else section in lines
            )
            if not section_found:
                violations.append(f"{relative_path} is missing required section {section!r}")

    for relative_path, markers in POLICY_DOC_MARKERS.items():
        content = read_text(ROOT / relative_path)
        if content is None:
            violations.append(f"{relative_path} is missing or unreadable")
            continue
        lowered = content.casefold()
        for marker in markers:
            if marker.casefold() not in lowered:
                violations.append(f"{relative_path} is missing required marker {marker!r}")

    for path in (ROOT / "docs").rglob("*.md"):
        if should_skip(path):
            continue
        content = read_text(path)
        if content is None:
            continue
        lowered = content.casefold()
        relative_path = path.relative_to(ROOT).as_posix()
        for phrase in FORBIDDEN_EXTERNAL_TRUTH_WORDING:
            if phrase in lowered:
                violations.append(
                    f"{relative_path} contains prohibited external-truth wording {phrase!r}"
                )
    return violations


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


def scan_rust_lint_bypasses() -> list[str]:
    violations = []
    for source in ROOT.rglob("*.rs"):
        if should_skip(source):
            continue
        content = read_text(source)
        if content is None:
            continue
        if any(re.search(pattern, content) for pattern in FORBIDDEN_RUST_LINT_BYPASSES):
            violations.append(str(source.relative_to(ROOT)))
    return violations


def scan_rust_forbidden_methods() -> list[str]:
    violations = []
    for source in ROOT.rglob("*.rs"):
        if should_skip(source):
            continue
        content = read_text(source)
        if content is None:
            continue
        for pattern, description in FORBIDDEN_RUST_METHODS:
            if re.search(pattern, content):
                violations.append(f"{source.relative_to(ROOT)} contains {description}")
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
    violations.extend(scan_documentation_contract())
    violations.extend(scan_module_sizes())
    violations.extend(
        f"{path} contains a Rust lint-bypass attribute" for path in scan_rust_lint_bypasses()
    )
    violations.extend(scan_rust_forbidden_methods())

    if violations:
        print("philosophy-check failed:")
        for violation in violations:
            print(f" - {violation}")
        return 1

    print("philosophy-check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
