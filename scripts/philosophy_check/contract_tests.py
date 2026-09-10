"""Contract tests rule family."""

from __future__ import annotations

from . import shared
from .shared import (
    _top_level_module_declarations,
    production_lib_paths,
    read_text,
    should_skip,
)
import re

RESPONSIBILITY_MAPS: dict[str, tuple[str, ...]] = {
    "src/lib.rs": (
        "version",
    ),
    "web/src/lib.rs": (
        "api",
        "api_types",
        "ask",
        "app",
        "components",
        "drafts",
        "index",
        "index_types",
        "markdown",
        "nav",
        "pages",
        "repositories",
        "repository_index_types",
        "repository_tree",
        "retrieval",
        "route",
        "search",
        "session",
        "state",
        "tasks",
    ),
    "crates/kernel/maestria-ports/src/traits.rs": (
        "errors",
        "repositories",
        "lifecycle",
        "indexing",
        "embedding",
        "provider_transport",
        "harness",
        "graph",
        "web",
        "approval",
        "search",
    ),
    "crates/kernel/maestria-ports/src/lib.rs": (
        "version",
        "execution",
        "learned_sparse",
        "learned_sparse_observations",
        "lexical",
        "full_text",
        "traits",
        "visual",
        "ocr",
        "parsing",
        "text",
        "validation",
        "in_memory",
        "contract_tests",
        "graph_contract_tests",
        "learned_sparse_contract_tests",
        "ocr_contract_tests",
        "visual_contract_tests",
        "multivector",
    ),
    "crates/kernel/maestria-domain/src/lib.rs": (
        "approval_outcome",
        "effects",
        "evidence_source",
        "entities",
        "errors",
        "events",
        "evidence_pack",
        "federated_evidence_bounds",
        "generations",
        "grant_token_digest",
        "ids",
        "input",
        "inputs",
        "kernel_state",
        "notebook_inputs",
        "model_agent",
        "notebook",
        "ocr",
        "provenance",
        "replay",
        "search",
        "realm_identity",
        "security",
        "security_snapshot",
        "sparse_namespace",
        "task_status",
        "types",
    ),
    "crates/kernel/maestria-governance/src/lib.rs": (
        "approval",
        "autonomy",
        "federation",
        "memory",
        "plan_validation",
        "privacy_exclusions",
        "prompt_injection",
        "retrieval",
        "risk",
        "scope",
        "secret_scanning",
        "validation",
    ),
    "crates/runtime/maestria-runtime/src/lib.rs": (
        "effect_executor_shutdown",
        "config",
        "effect_admission",
        "effect_dispatch",
        "effect_execution",
        "effect_execution_dispatch",
        "effect_result",
        "harness",
        "harness_gate",
        "indexing",
        "notebook_draft",
        "ocr",
        "parser_mapping",
        "parsing",
        "parsing_records",
        "persistence",
        "persistence_barrier",
        "proposal_recovery",
        "proposal_persistence",
        "proposal_workflow",
        "shell_policy",
        "supervision",
        "validation",
        "vector_indexing",
        "web_evidence",
        "parsing_terminal",
        "approval",
        "completion",
        "runtime",
        "runtime_effects",
        "runtime_handle",
        "runtime_loop",
        "runtime_transition",
    ),
    # ── core ──────────────────────────────────────────────────────────
    "crates/core/maestria-core/src/lib.rs": (
        "error",
        "evidence_opening",
        "ingestion",
        "instance",
        "manifest",
        "manifest_scope",
        "metrics",
        "notebook_draft_opening",
        "ports",
        "provenance",
        "types",
    ),
    "crates/apps/maestria-cli/src/lib.rs": (
        "test_support",
    ),
    "crates/apps/maestria-studio/src/lib.rs": (
        "agent",
        "http",
        "server",
    ),
    "crates/apps/maestria-daemon/src/lib.rs": (
        "api",
        "lock",
        "search_executor",
        "approval_recovery",
        "projection_recovery",
        "projection_watermark",
        "vector_startup",
        "full_text_recovery",
        "parser_resume",
        "recovery_inputs",
        "supervision_recovery",
        "validation_recovery",
        "lifecycle",
        "mutation_session",
        "watcher",
        "lifecycle_entry",
        "instance_setup",
        "providers",
        "learned_sparse_benchmark_executor",
        "sparse_startup",
        "runtime_construction",
        "blocked_patterns",
        "db_retry",
        "evidence_open",
        "source_identity",
        "notebook_draft_open",
        "projection_open",
        "recovery_staging",
        "repository_source_registration",
    ),
    "crates/apps/maestria-daemon/src/api.rs": (
        "protocol",
        "server",
        "services",
        "token",
    ),
    # -- storage ------------------------------------------------------
    "crates/storage/maestria-sqlite-support/src/lib.rs": (
        "connection",
        "db_retry",
        "error",
        "ids",
        "security",
    ),
    "crates/storage/maestria-storage-sqlite/src/lib.rs": (
        "db_retry",
        "events",
        "id_allocator",
        "journal",
        "late_interaction_io",
        "learned_sparse_io",
        "learned_sparse_projection",
        "payloads",
        "projection_cleanup",
        "repositories",
        "schema",
        "schema_validation",
        "sqlite_store",
    ),
    "crates/storage/maestria-search-tantivy/src/lib.rs": (
        "constructors",
        "error",
        "keys",
        "lexical_operations",
        "migration",
        "operations",
        "operations_cards",
        "operations_chunks",
        "schema",
        "scoring",
        "search_helpers",
        "documents",
        "tantivy_index",
        "execution",
    ),
    "crates/harness/maestria-harness/src/lib.rs": (
        "adapter",
        "command",
        "process",
        "tokenize",
    ),
    "crates/storage/maestria-blob-fs/src/lib.rs": (
        "store",
    ),
    "crates/storage/maestria-graph-sqlite/src/lib.rs": (
        "conversion",
        "migration",
        "graph",
    ),
    "crates/storage/maestria-vector-sqlite/src/lib.rs": (
        "encoding",
        "schema",
        "operations",
        "vector_index",
    ),
    # ── ecosystem ─────────────────────────────────────────────────────
    "crates/ecosystem/maestria-adapter-http/src/lib.rs": (
        "client",
        "helpers",
    ),
    "crates/ecosystem/maestria-memory/src/lib.rs": (
        "memory_service",
    ),
    "crates/ecosystem/maestria-ocr-local/src/lib.rs": (
        "rasterizer",
        "transport",
        "ocr_provider",
    ),
    "crates/ecosystem/maestria-web-evidence/src/lib.rs": (
        "web_fetcher",
    ),
    "crates/ecosystem/maestria-embedding-openai/src/lib.rs": (
        "embedding_provider",
    ),
    "crates/ecosystem/maestria-visual-local/src/lib.rs": (
        "dto",
        "visual_provider",
    ),
    "crates/ecosystem/maestria-sparse-local/src/lib.rs": (
        "dto",
        "sparse_provider",
    ),
    "crates/ecosystem/maestria-late-local/src/lib.rs": (
        "dto",
        "late_provider",
    ),
    "crates/ecosystem/maestria-retrieval/src/lib.rs": (
        "adapters",
        "benchmark_common",
        "bounded_reranker",
        "cancellation",
        "diversity",
        "engine",
        "fusion",
        "golden",
        "late_interaction_benchmark",
        "late_interaction_profile",
        "late_interaction_reranker",
        "late_interaction_scoring",
        "learned_sparse_benchmark",
        "learned_sparse_corpus",
        "learned_sparse_policy",
        "repository_benchmark",
        "rewrite",
        "traits",
        "types",
        "visual_benchmark",
        "visual_reranker",
        "monotonic",
    ),
    "crates/test-support/maestria-test-support/src/lib.rs": (
        "error",
        "git",
        "fs",
        "fixtures",
    ),
    "crates/ecosystem/maestria-code-intel/src/lib.rs": (
        "builder",
        "changes",
        "context",
        "context_assembly",
        "context_support",
        "delta",
        "error",
        "freshness",
        "identity",
        "incremental",
        "markers",
        "language",
        "metadata",
        "query",
        "references",
        "symbols",
        "types",
        "index",
        "walk",
        "selection",
    ),
    "crates/ecosystem/maestria-parsers/src/lib.rs": (
        "cargo_toml",
        "chunking",
        "generic_text",
        "markdown",
        "pdf",
        "pdf_geometry",
        "pdf_layout",
        "pdf_tree",
        "plain_text",
        "python_source",
        "registry",
        "rust_source",
        "tree_builder",
        "typescript_source",
    ),
    "crates/ecosystem/maestria-index-selection/src/lib.rs": (
        "policy",
        "scan",
        "classify",
        "candidates",
        "repo",
        "profile",
    ),
    "crates/ecosystem/maestria-validation/src/lib.rs": (
        "runner",
        "search_provenance",
        "search_security",
        "search_validators",
        "types",
        "validators",
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


def scan_responsibility_maps() -> list[str]:
    violations = []
    header_pattern = re.compile(r"^//[!|/] Responsibility map:\s*$")
    bullet_pattern = re.compile(r"^//[!|/]\s*-\s*`([^`]+)`\s*:")
    for rel_path, declared_modules in RESPONSIBILITY_MAPS.items():
        source = shared.ROOT / rel_path
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

        declared_mods = _top_level_module_declarations(content)
        for module in sorted(declared_mods - set(declared_modules)):
            violations.append(f"{rel_path} responsibility map omits module '{module}'")
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
    configured_maps = set(RESPONSIBILITY_MAPS)
    for source in production_lib_paths():
        rel_path = source.relative_to(shared.ROOT).as_posix()
        if rel_path not in configured_maps:
            violations.append(
                f"{rel_path} production module has no configured responsibility map"
            )
    return violations
