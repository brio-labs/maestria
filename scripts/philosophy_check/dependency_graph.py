"""Dependency graph rule family."""

from __future__ import annotations

from . import shared
from .shared import (
    _normalize_dependency_name,
    _toml_document,
    _workspace_dependency_specifications,
    _kernel_rust_files,
    _matching_delimiter,
    _rust_syntax,
    _scrubbed_production,
    _scrubbed_source,
    production_lib_paths,
    read_text,
)
import re
import json
import subprocess

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
    "std::env",
    "std::net",
    "std::thread",
    # Rule 1: deterministic domain code has no random sampling or hidden
    # global state; these tokens are defense-in-depth alongside the kernel
    # dependency whitelist (a rand-family crate cannot be declared, but a
    # std-level random source must not sneak in either).
    "thread_rng",
    "getrandom",
    "fastrand",
    "RandomState",
    # Uninitialized memory and raw interior mutability are never needed in
    # deterministic kernel code: `MaybeUninit` invites uninitialized reads and
    # `UnsafeCell` bypasses the borrow checker (Rule 1/21).
    "MaybeUninit",
    "UnsafeCell",
]


FORBIDDEN_KERNEL_PATTERNS = (
    (r"\bunsafe\s+(?:fn|impl|trait)\b|\bunsafe\s*\{", "unsafe Rust"),
)


FORBIDDEN_DOMAIN_FAILURES = [
    "unwrap(",
    "expect(",
    "panic!(",
    "unreachable!(",
    "todo!(",
    "unimplemented!(",
]


# Crates that own a user-facing console; `println!`-family writes are their
# legitimate interface instead of a logging violation.
APP_CRATE_DIRS = {
    "maestria-cli",
    "maestria-daemon",
    "maestria-harness-cli",
    "maestria-tui",
    "maestria-web",
}


KERNEL_ALLOWED_DEPENDENCIES = {
    "maestria-domain": {"serde", "serde-json", "sha2"},
    "maestria-governance": {"maestria-domain"},
    "maestria-ports": {"maestria-domain", "serde", "maestria-test-support"},
}








def _workspace_dependency_aliases(content: str | None) -> dict[str, str]:
    """Return normalized dependency aliases to their resolved package names."""
    aliases: dict[str, str] = {}
    for alias, specification in _workspace_dependency_specifications(content).items():
        package = specification.get("package", alias)
        aliases[alias] = _normalize_dependency_name(str(package))
    return aliases


def _manifest_dependencies(
    content: str,
    workspace_content: str | None = None,
) -> set[str]:
    document = _toml_document(content)
    if not document:
        return set()

    workspace_aliases = _workspace_dependency_aliases(workspace_content)
    dependencies: set[str] = set()

    def collect(table: object) -> None:
        if not isinstance(table, dict):
            return
        for dependency, specification in table.items():
            alias = _normalize_dependency_name(str(dependency))
            if isinstance(specification, dict) and specification.get("workspace") is True:
                # Cargo resolves an inherited dependency by the root alias.  The
                # root `package` field is authoritative for renamed packages.
                package = workspace_aliases.get(alias, alias)
            elif isinstance(specification, dict):
                package = specification.get("package", dependency)
            else:
                package = dependency
            dependencies.add(_normalize_dependency_name(str(package)))

    for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
        collect(document.get(table_name))
    targets = document.get("target", {})
    if isinstance(targets, dict):
        for target in targets.values():
            if not isinstance(target, dict):
                continue
            for table_name in (
                "dependencies",
                "dev-dependencies",
                "build-dependencies",
            ):
                collect(target.get(table_name))
    return dependencies


_KERNEL_IMPORT_PATTERN = re.compile(r"\buse\s+maestria_([a-z0-9_]+)")


def scan_kernel_imports() -> list[str]:
    """Enforce Rules 15/21 at source level: kernel `use` paths may only
    reference the crate itself or its declared kernel dependencies.

    Manifest checks (R10) catch dependency declarations; this catches
    cross-crate `use` paths directly, so an adapter import cannot sneak in
    through a renamed or inherited dependency.
    """
    violations = []
    for kernel_root in shared.KERNEL_ROOTS:
        crate_name = _normalize_dependency_name(kernel_root.name)
        allowed_imports = {
            _normalize_dependency_name(name)
            for name in KERNEL_ALLOWED_DEPENDENCIES.get(kernel_root.name, set())
            if _normalize_dependency_name(name).startswith("maestria-")
        }
        for source in (kernel_root / "src").rglob("*.rs"):
            rel = source.relative_to(shared.ROOT)
            production = _scrubbed_source(source)
            if not production:
                continue
            for raw in _KERNEL_IMPORT_PATTERN.findall(production):
                imported = _normalize_dependency_name(f"maestria_{raw}")
                if imported == crate_name or imported in allowed_imports:
                    continue
                violations.append(
                    f"{rel} imports forbidden kernel dependency maestria_{raw}"
                )
    return violations


def scan_kernel_manifests() -> list[str]:
    violations = []
    workspace_content = read_text(shared.ROOT / "Cargo.toml")
    for kernel_root in shared.KERNEL_ROOTS:
        manifest = kernel_root / "Cargo.toml"
        content = read_text(manifest)
        if content is None:
            violations.append(str(manifest.relative_to(shared.ROOT)))
            continue
        dependencies = _manifest_dependencies(content, workspace_content)
        crate_name = kernel_root.name
        allowed = KERNEL_ALLOWED_DEPENDENCIES.get(crate_name, set())
        for dependency in sorted(dependencies & FORBIDDEN_KERNEL_DEPENDENCIES):
            violations.append(
                f"{manifest.relative_to(shared.ROOT)} contains forbidden dependency token {dependency}"
            )
        for dependency in sorted(dependencies - allowed):
            if dependency in FORBIDDEN_KERNEL_DEPENDENCIES:
                continue
            violations.append(
                f"{manifest.relative_to(shared.ROOT)} contains disallowed kernel dependency {dependency}"
            )
    return violations


def _cargo_metadata_graph() -> dict[str, set[str]] | None:
    """Resolve the workspace dependency graph via `cargo metadata`.

    Returns package name -> transitive closure of its normal/build
    dependencies (package names, normalized). `None` when cargo is
    unavailable or the manifest does not resolve.
    """
    try:
        import json
        import subprocess

        result = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--offline"],
            capture_output=True,
            text=True,
            timeout=120,
            cwd=shared.ROOT,
        )
        if result.returncode != 0:
            return None
        document = json.loads(result.stdout)
    except (OSError, ValueError, subprocess.SubprocessError, json.JSONDecodeError):
        return None

    packages = document.get("packages", [])
    by_id: dict[str, str] = {}
    for package in packages:
        package_id = package.get("id")
        package_name = package.get("name")
        if isinstance(package_id, str) and isinstance(package_name, str):
            by_id[package_id] = _normalize_dependency_name(package_name)
    nodes = document.get("resolve", {}).get("nodes", [])
    edges: dict[str, set[str]] = {}
    for node in nodes:
        node_id = node.get("id")
        if not isinstance(node_id, str):
            continue
        deps = set()
        for dependency in node.get("deps", []):
            dep_id = dependency.get("pkg")
            if not isinstance(dep_id, str) or dep_id not in by_id:
                continue
            dep_kinds = dependency.get("dep_kinds", [])
            kinds = {
                kind.get("kind")
                for kind in dep_kinds
                if isinstance(kind, dict) and isinstance(kind.get("kind"), str)
            }
            if kinds and kinds == {"dev"}:
                continue
            deps.add(by_id[dep_id])
        edges[node_id] = deps

    closure: dict[str, set[str]] = {}
    for node_id, node_name in by_id.items():
        seen: set[str] = set()
        pending = list(edges.get(node_id, set()))
        while pending:
            dependency = pending.pop()
            if dependency in seen:
                continue
            seen.add(dependency)
            dependency_id = next(
                (candidate for candidate, name in by_id.items() if name == dependency),
                None,
            )
            if dependency_id is not None:
                pending.extend(edges.get(dependency_id, set()))
        closure[node_name] = seen
    return closure


def scan_kernel_dependency_closure() -> list[str]:
    """Rule 10/15: kernel crates stay dependency-layered transitively.

    Manifest checks cover direct declarations; this resolves the full graph
    with `cargo metadata` and verifies that no adapter/runtime/provider crate
    reaches a kernel crate through a path dependency or a transitive
    dependency of an allowed crate.
    """
    closure = _cargo_metadata_graph()
    if closure is None:
        return []
    violations = []
    for kernel_root in shared.KERNEL_ROOTS:
        crate_name = _normalize_dependency_name(kernel_root.name)
        allowed_direct = {
            _normalize_dependency_name(name)
            for name in KERNEL_ALLOWED_DEPENDENCIES.get(kernel_root.name, set())
        }
        allowed_closure = {
            dependency
            for allowed in allowed_direct
            for dependency in closure.get(allowed, set())
        } | allowed_direct
        for dependency in sorted(closure.get(crate_name, set()) - allowed_closure):
            if dependency in FORBIDDEN_KERNEL_DEPENDENCIES:
                violations.append(
                    f"{kernel_root.name} transitively depends on forbidden crate {dependency}"
                )
            else:
                violations.append(
                    f"{kernel_root.name} transitively depends on undisclosed crate "
                    f"{dependency}; declare it in KERNEL_ALLOWED_DEPENDENCIES or remove it"
                )
    return violations


def scan_kernel_sources() -> list[str]:
    violations = []
    for source in _kernel_rust_files(skip_tests=False, sorted_=True):
            content = read_text(source)
            if content is None:
                continue
            rel = source.relative_to(shared.ROOT)
            for token in FORBIDDEN_KERNEL_TOKENS:
                if token in content:
                    violations.append(f"{rel} contains forbidden kernel token {token}")
            for token in FORBIDDEN_DOMAIN_FAILURES:
                if token in content:
                    violations.append(f"{rel} contains forbidden failure token {token}")
            production = _scrubbed_production(source)
            for pattern, description in FORBIDDEN_KERNEL_PATTERNS:
                if re.search(pattern, production):
                    violations.append(f"{rel} contains forbidden {description}")
    return violations


def _rust_item_fragments(text: str) -> list[str]:
    """Split top-level Rust items without relying on keyword-shaped regexes."""
    source = _rust_syntax(text)
    fragments: list[str] = []
    index = 0
    while index < len(source):
        while index < len(source) and source[index].isspace():
            index += 1
        if index >= len(source):
            break
        if source.startswith("#[", index) or source.startswith("#![", index):
            opening = source.find("[", index)
            # Attributes may contain nested macro delimiters, so balance square
            # brackets directly instead of treating the attribute as an item.
            depth = 0
            cursor = opening
            while cursor != -1 and cursor < len(source):
                if source[cursor] == "[":
                    depth += 1
                elif source[cursor] == "]":
                    depth -= 1
                    if depth == 0:
                        cursor += 1
                        break
                cursor += 1
            if opening == -1 or depth:
                break
            index = cursor
            continue

        start = index
        stack: list[str] = []
        close_for = {")": "(", "]": "[", "}": "{"}
        ended = False
        while index < len(source):
            token = source[index]
            if token in "([{":
                if token == "{" and not stack:
                    prefix = source[start:index]
                    if re.match(r"\s*(?:pub(?:\s*\([^)]*\))?\s+)?use\b", prefix):
                        stack.append(token)
                    else:
                        closing = _matching_delimiter(source, index, "{", "}")
                        if closing is None:
                            index = len(source)
                        else:
                            index = closing + 1
                        fragments.append(source[start:index])
                        ended = True
                        break
                else:
                    stack.append(token)
            elif token in close_for and stack:
                if stack[-1] == close_for[token]:
                    stack.pop()
            elif token == ";" and not stack:
                index += 1
                fragments.append(source[start:index])
                ended = True
                break
            index += 1
        if not ended:
            if start < len(source):
                fragments.append(source[start:index])
            break
    return fragments


def _facade_item_allowed(fragment: str) -> bool:
    item = fragment.strip()
    if not item:
        return True
    # Visibility is deliberately required for reexports: ordinary implementation
    # imports belong in the implementation module, not in the façade.
    use_match = re.match(r"^(pub(?:\s*\([^)]*\))?)\s+use\b", item, re.DOTALL)
    if use_match is not None:
        use_source = _rust_syntax(item)
        return "*" not in use_source
    mod_match = re.match(
        r"^(?:(?:pub(?:\s*\([^)]*\))?|unsafe)\s+)*mod\s+[A-Za-z_][A-Za-z0-9_]*\s*;\s*$",
        item,
        re.DOTALL,
    )
    return mod_match is not None


def scan_facade_boundaries() -> list[str]:
    """Require every production library target to be a declarative façade."""
    violations = []
    for source in production_lib_paths():
        rel_path = source.relative_to(shared.ROOT).as_posix()
        content = read_text(source)
        if content is None:
            continue
        invalid_items = [
            fragment for fragment in _rust_item_fragments(content)
            if not _facade_item_allowed(fragment)
        ]
        if invalid_items:
            violations.append(
                f"{rel_path} contains {len(invalid_items)} implementation item(s); "
                f"implementation body/item detected (lib.rs should be a façade per Rule 19)"
            )
    return violations
