"""Contract tests rule family."""

from __future__ import annotations

import re
from pathlib import Path

from . import shared
from .shared import (
    RustModuleDeclaration,
    _production_rust_files,
    production_lib_paths,
    read_text,
    rust_module_declarations,
    should_skip,
)

RESPONSIBILITY_MAP_PATHS: frozenset[str] = frozenset(
    {
        "src/lib.rs",
        "web/src/lib.rs",
        "crates/kernel/sillage-ports/src/traits.rs",
        "crates/kernel/sillage-ports/src/lib.rs",
        "crates/kernel/sillage-domain/src/lib.rs",
        "crates/kernel/sillage-governance/src/lib.rs",
        "crates/runtime/sillage-runtime/src/lib.rs",
        "crates/core/sillage-core/src/lib.rs",
        "crates/apps/sillage-cli/src/lib.rs",
        "crates/apps/sillage-studio/src/lib.rs",
        "crates/apps/sillage-launcher/src/lib.rs",
        "crates/apps/sillage-daemon/src/lib.rs",
        "crates/apps/sillage-daemon/src/api.rs",
        "crates/storage/sillage-sqlite-support/src/lib.rs",
        "crates/storage/sillage-storage-sqlite/src/lib.rs",
        "crates/storage/sillage-search-tantivy/src/lib.rs",
        "crates/harness/sillage-harness/src/lib.rs",
        "crates/storage/sillage-blob-fs/src/lib.rs",
        "crates/storage/sillage-graph-sqlite/src/lib.rs",
        "crates/storage/sillage-vector-sqlite/src/lib.rs",
        "crates/ecosystem/sillage-adapter-http/src/lib.rs",
        "crates/ecosystem/sillage-memory/src/lib.rs",
        "crates/ecosystem/sillage-ocr-local/src/lib.rs",
        "crates/ecosystem/sillage-web-evidence/src/lib.rs",
        "crates/ecosystem/sillage-embedding-openai/src/lib.rs",
        "crates/ecosystem/sillage-extensions/src/lib.rs",
        "crates/ecosystem/sillage-visual-local/src/lib.rs",
        "crates/ecosystem/sillage-sparse-local/src/lib.rs",
        "crates/ecosystem/sillage-retrieval/src/lib.rs",
        "crates/test-support/sillage-test-support/src/lib.rs",
        "crates/ecosystem/sillage-code-intel/src/lib.rs",
        "crates/ecosystem/sillage-parsers/src/lib.rs",
        "crates/ecosystem/sillage-index-selection/src/lib.rs",
        "crates/ecosystem/sillage-validation/src/lib.rs",
    }
)

_RESPONSIBILITY_MAP_HEADER = re.compile(r"^//[!|/]\s*Responsibility map:\s*$")
_RESPONSIBILITY_MAP_ENTRY = re.compile(
    r"^//[!|/]\s*-\s*`([^`]+)`\s*:\s*(.*?)\s*$"
)


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
    "docs/ARCHITECTURE.md": (
        "## 2. System Identity",
        "## 3. Architectural Dependency Direction",
    ),
    "docs/SEARCH.md": (
        "## Search Boundary Objects",
        "## Search Execution Model",
        "## Budgets and Stop Conditions",
    ),
    "docs/MEMORY.md": (
        "## 1. Information Lifecycle",
        "## 2. Provenance and Staleness",
        "## 3. Boundaries and Overclaiming",
    ),
    "docs/SECURITY.md": (
        "## 2. Security Invariants",
        "## 5. Taint and Quarantine",
        "## 6. Prompt Injection as Data",
    ),
    "docs/OPERATIONS.md": (
        "## 1. Bounded Runtime Lifecycle",
        "## 2. State and Recovery",
        "## 4. Data Evolution",
    ),
    "docs/ROADMAP.md": ("## Milestone 1:", "## Milestone 4:"),
    "docs/RESEARCH.md": ("## 1. Evaluation Framework", "## 3. Promotion Criteria"),
}


POLICY_DOC_MARKERS = {
    "docs/PHILOSOPHY.md": (
        "41. Search plans",
        "42. Search traces",
        "43. Every retrieval lane",
        "44. Retrieval changes",
        "45. Normative architecture",
        "preserves external observations and provenance",
        "47. Model-generated search plans",
        "55. Learned-sparse retrieval",
        "56. Domain types own",
        "57. Production Rust uses",
        "58. Pure functional core separation",
        "59. Zero-copy and allocation discipline",
        "60. Deterministic fast collections",
    ),
    "docs/SPECS.md": (
        "I-Search-TypedBudgeted",
        "I-Search-TraceFingerprint",
        "I-Search-SecurityBeforeScore",
        "I-Search-Evaluated",
        "I-Domain-ValidStates",
    ),
}


FORBIDDEN_EXTERNAL_TRUTH_WORDING = (
    "domain owns truth",
    "truth machine",
    "truth store",
    "truth owner",
)


def scan_documentation_contract() -> list[str]:
    violations = []
    for relative_path, markers in CANONICAL_DOC_MARKERS.items():
        path = shared.ROOT / relative_path
        content = read_text(path)
        if content is None:
            violations.append(f"{relative_path} is missing or unreadable")
            continue
        lowered = content.casefold()
        for marker in markers:
            if marker.casefold() not in lowered:
                violations.append(
                    f"{relative_path} is missing required marker {marker!r}"
                )
        lines = {line.strip() for line in content.splitlines()}
        for section in CANONICAL_DOC_SECTIONS[relative_path]:
            section_found = (
                any(line.startswith(section) for line in lines)
                if section.endswith(":")
                else section in lines
            )
            if not section_found:
                violations.append(
                    f"{relative_path} is missing required section {section!r}"
                )

    for relative_path, markers in POLICY_DOC_MARKERS.items():
        content = read_text(shared.ROOT / relative_path)
        if content is None:
            violations.append(f"{relative_path} is missing or unreadable")
            continue
        lowered = content.casefold()
        for marker in markers:
            if marker.casefold() not in lowered:
                violations.append(
                    f"{relative_path} is missing required marker {marker!r}"
                )

    for path in (shared.ROOT / "docs").rglob("*.md"):
        if should_skip(path):
            continue
        content = read_text(path)
        if content is None:
            continue
        lowered = content.casefold()
        relative_path = path.relative_to(shared.ROOT).as_posix()
        for phrase in FORBIDDEN_EXTERNAL_TRUTH_WORDING:
            if phrase in lowered:
                violations.append(
                    f"{relative_path} contains prohibited external-truth wording {phrase!r}"
                )
    return violations


def responsibility_map_source_paths() -> tuple[Path, ...]:
    """Return policy sources, production facades, and header-bearing maps."""
    facades = set(production_lib_paths())
    sources = {
        shared.ROOT / relative_path
        for relative_path in RESPONSIBILITY_MAP_PATHS
    }
    sources.update(facades)
    for source in _production_rust_files(skip_tests=True, sorted_=True):
        content = read_text(source)
        if content is None:
            continue
        if any(
            _RESPONSIBILITY_MAP_HEADER.match(line)
            for line in content.splitlines()
        ):
            sources.add(source)
    return tuple(sorted(sources))


def scan_responsibility_maps() -> list[str]:
    violations: list[str] = []
    facades = set(production_lib_paths())
    required_paths = set(RESPONSIBILITY_MAP_PATHS)
    for source in responsibility_map_source_paths():
        rel_path = source.relative_to(shared.ROOT).as_posix()
        content = read_text(source)
        if content is None:
            violations.append(
                f"{rel_path} responsibility map source is missing or unreadable"
            )
            continue

        lines = content.splitlines()
        has_header = any(_RESPONSIBILITY_MAP_HEADER.match(line) for line in lines)
        entries: dict[str, tuple[str, int]] = {}
        for line_number, line in enumerate(lines, start=1):
            match = _RESPONSIBILITY_MAP_ENTRY.match(line)
            if match is None:
                continue
            module, description = match.groups()
            description = description.strip()
            if module in entries:
                first_line = entries[module][1]
                violations.append(
                    f"{rel_path}:{line_number} repeats responsibility entry "
                    f"for module '{module}' (first listed at line {first_line})"
                )
            else:
                entries[module] = (description, line_number)
            if not description:
                violations.append(
                    f"{rel_path}:{line_number} module '{module}' "
                    "has no responsibility description"
                )

        required_map = source in facades or rel_path in required_paths
        if not has_header:
            if required_map or entries:
                violations.append(f"{rel_path} has no responsibility map header")
            if not entries:
                continue

        declarations: dict[str, list[tuple[RustModuleDeclaration, int]]] = {}
        for declaration in rust_module_declarations(content):
            if (
                declaration.parent_modules
                or declaration.inline
                or declaration.test_only
            ):
                continue
            line_number = content.count("\n", 0, declaration.start) + 1
            declarations.setdefault(declaration.name, []).append(
                (declaration, line_number)
            )

        for module, module_declarations in declarations.items():
            if module not in entries:
                line_number = module_declarations[0][1]
                violations.append(
                    f"{rel_path}:{line_number} production module '{module}' "
                    "has no responsibility entry"
                )
        for module, (_description, line_number) in entries.items():
            if module not in declarations:
                violations.append(
                    f"{rel_path}:{line_number} responsibility entry names "
                    f"undeclared module '{module}'"
                )

        checked_source_paths: set[tuple[str, tuple[Path, ...]]] = set()
        for module, module_declarations in declarations.items():
            for declaration, line_number in module_declarations:
                if declaration.path_attribute:
                    if not declaration.path_supported or declaration.path is None:
                        continue
                    module_paths = (source.parent / declaration.path,)
                else:
                    module_dir = source.with_name(source.stem)
                    module_paths = (
                        module_dir / f"{module}.rs",
                        source.parent / f"{module}.rs",
                        module_dir / module / "mod.rs",
                        source.parent / module / "mod.rs",
                    )
                source_key = (module, module_paths)
                if source_key in checked_source_paths:
                    continue
                checked_source_paths.add(source_key)
                if not any(module_path.is_file() for module_path in module_paths):
                    source_detail = (
                        f" at '{declaration.path}'"
                        if declaration.path_attribute and declaration.path is not None
                        else ""
                    )
                    violations.append(
                        f"{rel_path}:{line_number} production module '{module}' "
                        f"has no source file{source_detail}"
                    )
    return violations