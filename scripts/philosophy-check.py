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
MODULE_SIZE_EXEMPTIONS: dict[str, str] = {
    "crates/apps/maestria-daemon/src/watcher.rs": "v0.7.0",
    "crates/apps/maestria-daemon/src/api/services.rs": "v0.7.0",
    "crates/kernel/maestria-domain/src/search_outcome.rs": "v0.7.0",
    "crates/ecosystem/maestria-retrieval/src/repository_benchmark.rs": "v0.7.0",
    "crates/ecosystem/maestria-retrieval/tests/contract_tests.rs": "v0.7.0",
}
ADR_MODULE_EXEMPTIONS: dict[str, str] = {
    "crates/apps/maestria-daemon/src/lib.rs": "v0.7.0",
    "crates/runtime/maestria-runtime/src/lib.rs": "v0.8.0",
    "crates/storage/maestria-storage-sqlite/src/lib.rs": "v0.7.0",
    "crates/storage/maestria-search-tantivy/src/lib.rs": "v0.7.0",
    "crates/storage/maestria-graph-sqlite/src/lib.rs": "v0.7.0",
    "crates/storage/maestria-vector-sqlite/src/lib.rs": "v0.7.0",
    "crates/harness/maestria-harness/src/lib.rs": "v0.7.0",
    "crates/core/maestria-core/src/lib.rs": "v0.8.0",
    "crates/kernel/maestria-governance/src/lib.rs": "v0.7.0",
    "crates/ecosystem/maestria-retrieval/src/lib.rs": "v0.7.0",
    "crates/ecosystem/maestria-code-intel/src/lib.rs": "v0.7.0",
    "crates/ecosystem/maestria-parsers/src/lib.rs": "v0.7.0",
}

VERSION_PATTERN = re.compile(r"^v?(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?$")


def parse_release_version(value: str) -> tuple[int, int, int] | None:
    match = VERSION_PATTERN.fullmatch(value.strip())
    if match is None:
        return None
    return tuple(int(part) for part in match.groups())


def workspace_version() -> str | None:
    manifest = read_text(ROOT / "Cargo.toml")
    if manifest is None:
        return None
    workspace_match = re.search(
        r"(?ms)^\[workspace\.package\]\s*(.*?)(?=^\[|\Z)",
        manifest,
    )
    if workspace_match is None:
        return None
    version_match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', workspace_match.group(1))
    return version_match.group(1) if version_match else None


def scan_exemption_expiry(current_version: str | None = None) -> list[str]:
    current_text = current_version or workspace_version()
    if current_text is None:
        return ["workspace Cargo.toml has no parseable [workspace.package] version"]
    current = parse_release_version(current_text)
    if current is None:
        return [f"workspace version {current_text!r} is not a supported release version"]

    violations = []
    exemptions = {**MODULE_SIZE_EXEMPTIONS, **ADR_MODULE_EXEMPTIONS}
    for path, target_text in sorted(exemptions.items()):
        target = parse_release_version(target_text)
        if target is None:
            violations.append(
                f"{path} has malformed module exemption expiry {target_text!r}"
            )
        elif current >= target:
            violations.append(
                f"{path} module exemption expired at {target_text} "
                f"(workspace version {current_text}); refactor or renew the ADR"
            )
    return violations


KERNEL_ALLOWED_DEPENDENCIES = {
    "maestria-domain": {"sha2"},
    "maestria-governance": {"maestria_domain"},
    "maestria-ports": {"maestria_domain"},
}
RESPONSIBILITY_MAPS: dict[str, tuple[str, ...]] = {
    # ── kernel ───────────────────────────────────────────────────────
    "crates/kernel/maestria-ports/src/traits.rs": (
        "errors", "repositories", "lifecycle", "indexing", "embedding",
        "harness", "graph", "web", "approval", "search",
    ),
    "crates/kernel/maestria-domain/src/lib.rs": (
        "effects", "entities", "errors", "events", "evidence_pack",
        "generations", "ids", "input", "inputs", "kernel_state",
        "provenance", "replay", "search", "security", "types",
    ),
    "crates/kernel/maestria-governance/src/lib.rs": (
        "approval", "autonomy", "memory", "plan_validation", "privacy",
        "retrieval", "risk", "scope", "validation",
    ),
    # ── runtime ──────────────────────────────────────────────────────
    "crates/runtime/maestria-runtime/src/lib.rs": (
        "config", "effect_dispatch", "effect_execution", "effect_result",
        "harness", "indexing", "parser_mapping", "parsing",
        "parsing_records", "persistence", "shell_policy", "supervision",
        "validation", "vector_indexing", "web_evidence", "approval",
        "completion",
    ),
    # ── core ──────────────────────────────────────────────────────────
    "crates/core/maestria-core/src/lib.rs": (
        "error", "evidence_opening", "evidence_pack_provenance",
        "ingestion", "instance", "manifest", "ports", "provenance", "types",
    ),
    "crates/apps/maestria-daemon/src/lib.rs": (
        "api", "lock", "search_executor", "approval_recovery", "projection_recovery",
        "vector_startup", "full_text_recovery", "parser_resume",
        "recovery_inputs", "supervision_recovery", "validation_recovery",
        "lifecycle", "watcher",
    ),
    "crates/apps/maestria-daemon/src/api.rs": (
        "protocol", "server", "services", "token",
    ),
    # ── harness ───────────────────────────────────────────────────────
    "crates/harness/maestria-harness/src/lib.rs": (
        "command", "process", "tokenize",
    ),
    # ── storage ───────────────────────────────────────────────────────
    "crates/storage/maestria-storage-sqlite/src/lib.rs": (
        "events", "id_allocator", "payloads", "repositories",
        "schema", "schema_validation",
    ),
    "crates/storage/maestria-search-tantivy/src/lib.rs": (
        "constructors", "lexical_helpers", "lexical_operations",
        "migration", "operations", "schema", "search_helpers",
    ),
    "crates/storage/maestria-graph-sqlite/src/lib.rs": (
        "conversion", "migration",
    ),
    "crates/storage/maestria-vector-sqlite/src/lib.rs": (
        "encoding", "schema",
    ),
    # ── ecosystem ─────────────────────────────────────────────────────
    "crates/ecosystem/maestria-retrieval/src/lib.rs": (
        "adapters", "bounded_reranker", "diversity", "engine", "fusion",
        "golden", "learned_sparse_benchmark", "learned_sparse_policy",
        "repository_benchmark", "rewrite", "sync", "sync_engine", "traits",
        "types", "visual_benchmark", "visual_reranker",
    ),
    "crates/ecosystem/maestria-code-intel/src/lib.rs": (
        "builder", "context", "context_assembly", "context_support",
        "error", "freshness", "identity", "metadata", "query",
        "symbols", "types",
    ),
    "crates/ecosystem/maestria-parsers/src/lib.rs": (
        "cargo_toml", "chunking", "markdown", "pdf", "pdf_layout",
        "plain_text", "registry", "rust_source", "tree_builder",
    ),
    "crates/ecosystem/maestria-validation/src/lib.rs": (
        "runner", "search_provenance", "search_security",
        "search_validators", "types", "validators",
    ),
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
        "55. Learned-sparse retrieval",
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
        if is_test_source(source):
            continue
        production = production_rust(content)
        for pattern, description in FORBIDDEN_RUST_METHODS:
            if re.search(pattern, production):
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
def scan_responsibility_maps() -> list[str]:
    violations = []
    header_pattern = re.compile(r"^//[!|/] Responsibility map:\s*$")
    bullet_pattern = re.compile(r"^//[!|/]\s*-\s*`([^`]+)`\s*:")
    for rel_path, declared_modules in RESPONSIBILITY_MAPS.items():
        source = ROOT / rel_path
        content = read_text(source)
        if content is None:
            violations.append(f"{rel_path} responsibility map file not present")
            continue

        lines = content.splitlines()
        if not any(header_pattern.match(line) for line in lines):
            violations.append(f"{rel_path} lacks a responsibility map header")
            continue

        observed_modules = tuple(
            match.group(1)
            for line in lines
            for match in [bullet_pattern.match(line)]
            if match is not None
        )
        if observed_modules != declared_modules:
            for module in declared_modules:
                if module not in observed_modules:
                    violations.append(
                        f"{rel_path} responsibility map is missing module '{module}'"
                    )
            for module in observed_modules:
                if module not in declared_modules:
                    violations.append(
                        f"{rel_path} responsibility map has extra module '{module}'"
                    )

        declared_pattern = re.compile(r"^(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+([a-z0-9_]+)\s*;\s*$")
        declared_mods = set(
            match.group(1)
            for line in lines
            for match in [declared_pattern.match(line)]
            if match is not None
        )
        for module in declared_modules:
            if module not in declared_mods:
                violations.append(f"{rel_path} does not declare module '{module}'")
            module_dir = source.with_name(source.stem)
            module_path = module_dir / f"{module}.rs"
            if not module_path.exists():
                legacy_path = source.parent / f"{module}.rs"
                if not legacy_path.exists():
                    # Check directory module: module/<mod.rs> or <stem>/module/<mod.rs>
                    dir_mod = module_dir / module / "mod.rs"
                    if not dir_mod.exists():
                        # For lib.rs files, check source.parent/<module>/mod.rs
                        alt_dir_mod = source.parent / module / "mod.rs"
                        if not alt_dir_mod.exists():
                            violations.append(
                                f"{rel_path} responsibility module file missing: {module}.rs"
                            )
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
        if rel in MODULE_SIZE_EXEMPTIONS or rel in ADR_MODULE_EXEMPTIONS:
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


def production_strip_line_comments(body: str) -> str:
    """Remove single-line `//` comments (but not `//!` doc-comments)."""
    lines = []
    for line in body.splitlines(keepends=True):
        stripped = line.lstrip()
        if stripped.startswith("// ") or stripped.startswith("//\n") or stripped.startswith("//\r"):
            lines.append("\n")
        elif stripped.startswith("/*"):
            lines.append("\n")
        else:
            lines.append(line)
    return "".join(lines)


def scan_facade_boundaries() -> list[str]:
    """Verify that lib.rs files act as façades (Rule 19).

    A lib.rs should only contain module declarations, re-exports, and
    metadata constants. Implementation bodies (fn, struct, enum, impl blocks)
    indicate accumulated responsibility and should be extracted to sibling
    modules.
    """
    violations = []
    # Check for fn definitions (handles pub, pub(crate), async, pub async)
    fn_pat = re.compile(
        r'^\s*(?:pub(?:\s*\([^)]*\))?\s+)?(?:async\s+)?fn\s+\w+\s*\(',
        re.MULTILINE,
    )
    # Check for struct/enum definitions with bodies
    se_pat = re.compile(
        r'^\s*(?:pub\s+)?(?:struct|enum)\s+\w+(?:\s*<[^>]*>)?\s*(?::\s*[^{;]+)?\{',
        re.MULTILINE,
    )
    # Check for impl blocks
    impl_pat = re.compile(
        r'^\s*(?:pub\s+)?(?:unsafe\s+)?impl\s+(?:<[^>]*>\s*)?\w+(?:::\w+)?'
        r'(?:\s*<[^>]*>)?(?:\s+for\s+\w+(?:::\w+)?(?:\s*<[^>]*>)?)?\{',
        re.MULTILINE,
    )
    # Check for const/static definitions
    const_pat = re.compile(
        r'^\s*(?:pub\s+)?(?:const|static)\s+\w+\s*(?::|=)',
        re.MULTILINE,
    )
    for rel_path in RESPONSIBILITY_MAPS:
        source = ROOT / rel_path
        if not source.exists():
            continue
        if rel_path in ADR_MODULE_EXEMPTIONS:
            continue
        content = read_text(source)
        if content is None:
            continue

        # Remove line comments so doc-comment //! descriptions don't match
        cleaned = production_strip_line_comments(content)

        hits = (fn_pat.findall(cleaned)
                + se_pat.findall(cleaned)
                + impl_pat.findall(cleaned)
                + const_pat.findall(cleaned))
        if hits:
            violations.append(
                f"{rel_path} contains {len(hits)} implementation body(s) "
                f"(lib.rs should be a façade per Rule 19)"
            )
    return violations


def scan_cohesion() -> list[str]:
    """Emit cohesion / concept-density signals for large façade modules.

    Flags lib.rs files where the ratio of logical lines per declared module
    exceeds a heuristic threshold, indicating a single file may be carrying
    too many responsibilities.
    """
    violations = []
    for rel_path, declared_modules in RESPONSIBILITY_MAPS.items():
        source = ROOT / rel_path
        if rel_path in ADR_MODULE_EXEMPTIONS:
            continue
        if not source.exists():
            continue
        content = read_text(source)
        if content is None:
            continue
        production = content.split("#[cfg(test)]", 1)[0]
        logical = logical_line_count(production)
        num_modules = len(declared_modules)
        if num_modules == 0:
            continue
        density = logical / num_modules
        # If there are more than 15 logical lines per declared module in the
        # lib.rs, the façade is likely accumulating responsibility that
        # belongs in sibling modules.
        if density > 15.0:
            violations.append(
                f"{rel_path} has {density:.1f} logical lines per module "
                f"({logical} lines across {num_modules} modules) — "
                f"cohesion signal: extract implementation to modules"
            )
    return violations
def main() -> int:
    violations = []
    marker_violations = scan_markers()
    violations.extend(f"{path} contains forbidden task marker" for path in marker_violations)
    violations.extend(scan_kernel_manifests())
    violations.extend(scan_kernel_sources())
    violations.extend(scan_documentation_contract())
    violations.extend(scan_responsibility_maps())
    violations.extend(scan_module_sizes())
    violations.extend(scan_exemption_expiry())
    violations.extend(
        f"{path} contains a Rust lint-bypass attribute" for path in scan_rust_lint_bypasses()
    )
    violations.extend(scan_rust_forbidden_methods())
    violations.extend(scan_facade_boundaries())
    violations.extend(scan_cohesion())

    if violations:
        print("philosophy-check failed:")
        for violation in violations:
            print(f" - {violation}")
        return 1

    print("philosophy-check passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
