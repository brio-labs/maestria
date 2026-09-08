"""Shared paths, source iteration, and Rust text helpers."""

from __future__ import annotations

import re
from collections.abc import Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


THIS_SCRIPT = ROOT / "scripts" / "philosophy-check.py"

# The checker's own package carries pattern literals (marker words, secret
# shapes) that must never trip the scans they define.
PACKAGE_DIR = Path(__file__).resolve().parent


def _is_checker_source(path: Path) -> bool:
    resolved = path.resolve()
    return resolved == THIS_SCRIPT or resolved.parent == PACKAGE_DIR


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


# Files exempted from the doctrine scan (e.g. vendored or legacy artifacts).
SKIP_FILES = set()


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


def should_skip(path: Path) -> bool:
    rel = path.relative_to(ROOT)
    rel_parts = set(rel.parts)
    return (
        _is_checker_source(path)
        or rel.as_posix() in SKIP_FILES
        or bool(rel_parts.intersection(SKIP_DIRS))
    )


def _production_rust_files(*, skip_tests: bool = True, sorted_: bool = False) -> Iterator[Path]:
    iterator = sorted(ROOT.rglob("*.rs")) if sorted_ else ROOT.rglob("*.rs")
    for path in iterator:
        if should_skip(path):
            continue
        if skip_tests and is_test_source(path):
            continue
        yield path


def _kernel_rust_files(*, skip_tests: bool = True, sorted_: bool = False) -> Iterator[Path]:
    for kernel_root in KERNEL_ROOTS:
        files = (kernel_root / "src").rglob("*.rs")
        if sorted_:
            files = sorted(files)
        for path in files:
            if skip_tests and is_test_source(path):
                continue
            yield path


def _iter_scannable(root: Path) -> Iterator[Path]:
    """Walk *root*, pruning SKIP_DIRS at walk time.

    Filtering with `should_skip` per candidate called `Path.resolve()` for
    every entry (including the 190k+ entries under `target/` and `.git`),
    which dominated the checker runtime; pruning by directory name avoids
    descending into them at all.
    """
    pending = [root]
    while pending:
        current = pending.pop()
        try:
            entries = sorted(current.iterdir(), key=lambda entry: entry.name)
        except OSError:
            continue
        for entry in entries:
            if entry.name in SKIP_DIRS:
                continue
            if entry.is_dir():
                pending.append(entry)
            elif entry.is_file() and entry.name not in SKIP_FILES:
                yield entry


def read_text(path: Path) -> str | None:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


_SCRUB_CACHE: dict[tuple[Path, int, int], str] = {}


_SCRUB_PRODUCTION_CACHE: dict[tuple[Path, int, int], str] = {}


def _scrubbed_source(path: Path) -> str:
    """Return syntax-scrubbed source, cached per (path, mtime, size).

    Several scans lex every Rust file independently; the scrubber is a
    character-by-character Python pass, so sharing one scrubbed copy per
    file avoids re-lexing the same content for each rule.
    """
    try:
        stat = path.stat()
    except OSError:
        return ""
    key = (path, stat.st_mtime_ns, stat.st_size)
    cached = _SCRUB_CACHE.get(key)
    if cached is not None:
        return cached
    content = read_text(path)
    if content is None:
        return ""
    scrubbed = _rust_syntax(content)
    if len(_SCRUB_CACHE) < 4096:
        _SCRUB_CACHE[key] = scrubbed
    return scrubbed


def _scrubbed_production(path: Path) -> str:
    """Like [`_scrubbed_source`], but truncated at `#[cfg(test)]` so test
    modules inside production files do not count toward production budgets."""
    try:
        stat = path.stat()
    except OSError:
        return ""
    key = (path, stat.st_mtime_ns, stat.st_size)
    cached = _SCRUB_PRODUCTION_CACHE.get(key)
    if cached is not None:
        return cached
    content = read_text(path)
    if content is None:
        return ""
    scrubbed = _rust_syntax(production_rust(content))
    if len(_SCRUB_PRODUCTION_CACHE) < 4096:
        _SCRUB_PRODUCTION_CACHE[key] = scrubbed
    return scrubbed


def _matching_delimiter(text: str, opening: int, left: str, right: str) -> int | None:
    depth = 0
    for index in range(opening, len(text)):
        token = text[index]
        if token == left:
            depth += 1
        elif token == right:
            depth -= 1
            if depth == 0:
                return index
    return None


def _balanced_end(text: str, opening: int, left: str, right: str) -> int:
    """End (exclusive) of the balanced `left`/`right` group starting at
    *opening* (which points at `left`)."""
    idx = _matching_delimiter(text, opening, left, right)
    return idx + 1 if idx is not None else len(text)


def _gated_item_end(text: str, attr_start: int) -> int:
    """End (exclusive) of the item gated by the attribute at *attr_start*.

    Consumes the attribute, any immediately following attributes, then the
    item body: a balanced `{...}` group or a declaration ending at a
    top-level `;`. The scan runs on syntax-scrubbed text so braces inside
    strings or comments cannot confuse item boundaries.
    """
    n = len(text)
    index = attr_start
    while index < n:
        if text[index] == "#" and index + 1 < n and text[index + 1] == "[":
            index = _balanced_end(text, index + 1, "[", "]")
            continue
        break
    while index < n and text[index] in " \t\r\n":
        index += 1
    depths = {"{": 0, "(": 0, "[": 0}
    closing = {"}": "{", ")": "(", "]": "["}
    for idx in range(index, n):
        token = text[idx]
        if token in depths:
            depths[token] += 1
        elif token in closing and depths[closing[token]] > 0:
            depths[closing[token]] -= 1
            if token == "}" and not any(depths.values()):
                return idx + 1
        elif token == ";" and not any(depths.values()):
            return idx + 1
    return n


def _cfg_test_item_extents(text: str) -> list[tuple[int, int]]:
    """Extents of every `#[cfg(test)]`-gated item in *text*.

    Offsets are located in syntax-scrubbed text so occurrences inside
    comments or string literals never trigger removal; the scrubber blanks
    those regions while preserving byte positions.
    """
    scrubbed = _rust_syntax(text)
    extents: list[tuple[int, int]] = []
    search_from = 0
    while True:
        start = scrubbed.find("#[cfg(test)]", search_from)
        if start == -1:
            return extents
        end = _gated_item_end(scrubbed, start)
        extents.append((start, end))
        search_from = end


def production_rust(text: str) -> str:
    """Production portion of a Rust file: the source with every
    `#[cfg(test)]`-gated item removed.

    Test-only imports at the top of a file, test modules, and gated helper
    functions are excluded while production code after them is preserved;
    the literal appearing inside a comment or string never truncates.
    """
    extents = _cfg_test_item_extents(text)
    if not extents:
        return text
    kept: list[str] = []
    cursor = 0
    for start, end in extents:
        kept.append(text[cursor:start])
        cursor = end
    kept.append(text[cursor:])
    return "".join(kept)


_FN_QUALIFIERS = r"(?:const\s+|async\s+|unsafe\s+|extern(?:\s+\"[^\"]*\")?\s+)*"


_NAMED_STRUCT_PATTERN = re.compile(
    r"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?struct\s+(\w+)\b",
    re.MULTILINE,
)


_FUNCTION_PARAMETER_PATTERN = re.compile(
    rf"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?{_FN_QUALIFIERS}fn\s+(\w+)\b",
    re.MULTILINE,
)


_PUBLIC_FUNCTION_PATTERN = re.compile(
    rf"^\s*pub(?:\s*\([^)]*\))?\s+{_FN_QUALIFIERS}fn\s+(\w+)\b",
    re.MULTILINE,
)


_FIELD_PREFIX = r"\s*(?:#\[[^\]]+\]\s*)*"


_FIELD_VIS = r"(?:pub(?:\s*\([^)]*\))?\s+)?"


_SIMPLE_FIELD_PATTERN = re.compile(
    rf"{_FIELD_PREFIX}{_FIELD_VIS}(\w+)\s*:\s*(.+?)\s*",
    re.DOTALL,
)


_SIMPLE_PARAMETER_PATTERN = re.compile(
    r"\s*(?:mut\s+)?(\w+)\s*:\s*(.+?)\s*",
    re.DOTALL,
)


def _blank_non_newlines(
    chars: list[str], start: int, end: int, replacement: str = " "
) -> None:
    for index in range(start, end):
        if chars[index] not in {"\n", "\r"}:
            chars[index] = replacement


def _rust_syntax(text: str, replacement: str = " ") -> str:
    """Blank comments and literals while preserving source positions."""
    chars = list(text)
    index = 0
    while index < len(text):
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = len(text) if end == -1 else end
            _blank_non_newlines(chars, index, end, replacement)
            index = end
            continue

        if text.startswith("/*", index):
            depth = 1
            end = index + 2
            while end < len(text) and depth:
                if text.startswith("/*", end):
                    depth += 1
                    end += 2
                elif text.startswith("*/", end):
                    depth -= 1
                    end += 2
                else:
                    end += 1
            _blank_non_newlines(chars, index, end, replacement)
            index = end
            continue

        raw = re.match(r"(?:br|r)(?P<hashes>#{0,255})\"", text[index:])
        if raw is not None:
            delimiter = '"' + raw.group("hashes")
            content_start = index + raw.end()
            closing = text.find(delimiter, content_start)
            end = len(text) if closing == -1 else closing + len(delimiter)
            _blank_non_newlines(chars, index, end, replacement)
            index = end
            continue

        if text[index] == '"':
            end = index + 1
            escaped = False
            while end < len(text):
                token = text[end]
                end += 1
                if escaped:
                    escaped = False
                elif token == "\\":
                    escaped = True
                elif token == '"':
                    break
            _blank_non_newlines(chars, index, end, replacement)
            index = end
            continue

        if text[index] == "'":
            lifetime = re.match(r"'[A-Za-z_][A-Za-z0-9_]*", text[index:])
            if lifetime is not None:
                lifetime_end = index + lifetime.end()
                if lifetime_end >= len(text) or text[lifetime_end] != "'":
                    index = lifetime_end
                    continue
            end = index + 1
            escaped = False
            while end < len(text):
                token = text[end]
                end += 1
                if escaped:
                    escaped = False
                elif token == "\\":
                    escaped = True
                elif token == "'":
                    break
            _blank_non_newlines(chars, index, end, replacement)
            index = end
            continue

        index += 1
    return "".join(chars)


def _item_opening(text: str, start: int, target: str) -> int | None:
    depths = {"<": 0, "(": 0, "[": 0}
    closing = {">": "<", ")": "(", "]": "["}
    for index in range(start, len(text)):
        token = text[index]
        if token == target and not any(depths.values()):
            return index
        if token == ";" and not any(depths.values()):
            return None
        if token in depths:
            depths[token] += 1
        elif token in closing:
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
        elif token == "{" and target != "{" and not any(depths.values()):
            return None
    return None


def _top_level_comma_items(text: str) -> list[str]:
    items = []
    start = 0
    depths = {"<": 0, "(": 0, "[": 0, "{": 0}
    closing = {">": "<", ")": "(", "]": "[", "}": "{"}
    for index, token in enumerate(text):
        if token in depths:
            depths[token] += 1
        elif token in closing:
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
        elif token == "," and not any(depths.values()):
            item = text[start:index].strip()
            if item:
                items.append(item)
            start = index + 1
    final = text[start:].strip()
    if final:
        items.append(final)
    return items


def _named_values(text: str, pattern: re.Pattern[str]) -> list[tuple[str, str]]:
    values = []
    for item in _top_level_comma_items(text):
        match = pattern.fullmatch(item)
        if match is not None:
            values.append((match.group(1), match.group(2).strip()))
    return values


def _named_struct_fields(text: str) -> list[tuple[str, list[tuple[str, str]]]]:
    return [
        (name, [(n, t) for n, t, _ in fields])
        for name, fields in _named_struct_fields_with_visibility(text)
    ]


def _function_parameters(text: str) -> list[tuple[str, list[tuple[str, str]]]]:
    functions = []
    for match in _FUNCTION_PARAMETER_PATTERN.finditer(text):
        opening = _item_opening(text, match.end(), "(")
        if opening is None:
            continue
        closing = _matching_delimiter(text, opening, "(", ")")
        if closing is None:
            continue
        parameters = _named_values(
            text[opening + 1 : closing], _SIMPLE_PARAMETER_PATTERN
        )
        functions.append((match.group(1), parameters))
    return functions


_PUBLIC_ASYNC_FN_PATTERN = re.compile(
    r"^\s*pub\s+async\s+fn\s+(\w+)\s*\(",
    re.MULTILINE,
)


_SERDE_TRY_FROM_PATTERN = re.compile(r"#\s*\[\s*serde\s*\([^]]*\btry_from\s*=")


_FALLIBLE_CONSTRUCTOR_PATTERN = re.compile(
    r"fn\s+(?:new|parse|from_canonical|try_new)\s*\([^)]*\)\s*->\s*Result\s*<\s*Self"
)


_TRY_FROM_IMPL_PATTERN = re.compile(
    r"impl\s+TryFrom\s*<[^>]*>\s+for\s+(\w+)\s*\{"
)


_FIELD_VISIBILITY_PATTERN = re.compile(
    rf"^{_FIELD_PREFIX}(?P<pub>{_FIELD_VIS})?(?P<name>\w+)\s*:\s*(?P<type>.+?)\s*$",
    re.MULTILINE,
)


def _named_struct_fields_with_visibility(
    text: str,
) -> list[tuple[str, list[tuple[str, str, bool]]]]:
    """Like [`_named_struct_fields`], but records whether each field is
    public so validating-constructor bypasses are mechanically detectable."""
    structs = []
    for match in _NAMED_STRUCT_PATTERN.finditer(text):
        opening = _item_opening(text, match.end(), "{")
        if opening is None:
            continue
        closing = _matching_delimiter(text, opening, "{", "}")
        if closing is None:
            continue
        clean_body = re.sub(r"#\s*\[[^\]]*\]", " ", text[opening + 1 : closing])
        raw_fields = []
        current = []
        depth = 0
        for ch in clean_body:
            if ch == "<":
                depth += 1
                current.append(ch)
            elif ch == ">":
                depth = max(0, depth - 1)
                current.append(ch)
            elif ch == "," and depth == 0:
                raw_fields.append("".join(current).strip())
                current = []
            elif ch == "\n" and depth == 0:
                current.append(" ")
            else:
                current.append(ch)
        if current:
            last = "".join(current).strip()
            if last:
                raw_fields.append(last)
        fields = []
        for rf in raw_fields:
            if not rf or ":" not in rf:
                continue
            parts = rf.split(":", 1)
            name_part = parts[0].strip()
            type_part = parts[1].strip().rstrip(",")
            is_pub = False
            if name_part.startswith("pub"):
                is_pub = True
                name_part = re.sub(r"^pub(?:\s*\([^)]*\))?\s+", "", name_part).strip()
            if re.fullmatch(r"[a-zA-Z_]\w*", name_part):
                fields.append((name_part, type_part, is_pub))
        structs.append((match.group(1), fields))
    return structs


def is_test_source(path: Path) -> bool:
    """True for test-only files and directories.

    Covers the `tests/` directory, `*_tests/` directories (e.g.
    `watcher_tests/`), inline module files named `tests_*.rs`, `*_test.rs`,
    `*_tests.rs`, `test_support.rs`, and `contract_tests/` modules so
    production-scoped scans never treat test code as shipped code.
    """
    return (
        "tests" in path.parts
        or any(segment.endswith("_tests") for segment in path.parts)
        or path.stem in {"tests", "contract_tests", "test_support"}
        or path.stem.endswith("_tests")
        or path.stem.endswith("_test")
        or path.stem.startswith("tests_")
        or (path.suffix == ".py" and path.stem.startswith("test_"))
    )


_MOD_DECL_PATTERN = re.compile(
    r"\bmod\s+([a-zA-Z_][a-zA-Z0-9_]*)\s*;",
)


def _top_level_module_declarations(text: str) -> set[str]:
    """Return top-level module declarations, excluding exact cfg(test) only."""
    source = _rust_syntax(text)
    pattern = _MOD_DECL_PATTERN
    declarations: set[str] = set()
    depth = 0
    cursor = 0
    for match in pattern.finditer(source):
        between = source[cursor : match.start()]
        for token in between:
            if token == "{":
                depth += 1
            elif token == "}":
                depth = max(0, depth - 1)
        cursor = match.end()
        if depth:
            continue
        prefix = source[: match.start()]
        tail = re.split(r"[;}]", prefix)[-1]
        attributes = re.findall(r"#\s*\[[^\]]*\]", tail)
        remainder = re.sub(r"#\s*\[[^\]]*\]", "", tail).strip()
        test_cfg = any(
            re.fullmatch(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", attribute)
            for attribute in attributes
        )
        if test_cfg and re.fullmatch(
            r"(?:pub(?:\s*\([^)]*\))?)?", remainder
        ):
            continue
        declarations.add(match.group(1))
    return declarations


def logical_line_count(content: str) -> int:
    """Count source lines that carry code.

    The syntax scrubber blanks comments and string/char literals, so block
    comments and multi-line strings do not inflate the budget and a code
    line with a trailing comment still counts exactly once.
    """
    return _logical_line_count_scrubbed(_rust_syntax(content))


def _logical_line_count_scrubbed(scrubbed: str) -> int:
    return sum(1 for line in scrubbed.splitlines() if line.strip())


def _workspace_member_roots() -> list[Path] | None:
    """Discover workspace packages from Cargo members, globs, excludes, and root."""
    manifest_path = ROOT / "Cargo.toml"
    content = read_text(manifest_path)
    if content is None:
        return None
    document = _toml_document(content)
    workspace = document.get("workspace")
    package = document.get("package")
    if not isinstance(workspace, dict) and not isinstance(package, dict):
        return None

    roots: set[Path] = set()
    if isinstance(package, dict):
        roots.add(ROOT)
    members = workspace.get("members", []) if isinstance(workspace, dict) else []
    if isinstance(members, str):
        members = [members]
    if isinstance(members, list):
        for member in members:
            if not isinstance(member, str):
                continue
            matches = list(ROOT.glob(member))
            if not matches and (ROOT / member).is_dir():
                matches = [ROOT / member]
            for match in matches:
                candidate = match.parent if match.name == "Cargo.toml" else match
                if candidate.is_dir() and (candidate / "Cargo.toml").is_file():
                    roots.add(candidate)

    excluded: set[Path] = set()
    excludes = workspace.get("exclude", []) if isinstance(workspace, dict) else []
    if isinstance(excludes, str):
        excludes = [excludes]
    if isinstance(excludes, list):
        for excluded_pattern in excludes:
            if not isinstance(excluded_pattern, str):
                continue
            matches = list(ROOT.glob(excluded_pattern))
            if not matches and (ROOT / excluded_pattern).exists():
                matches = [ROOT / excluded_pattern]
            for match in matches:
                excluded.add(match.parent if match.name == "Cargo.toml" else match)
    return sorted(
        root
        for root in roots
        if not any(root == excluded_root or excluded_root in root.parents for excluded_root in excluded)
    )


def _path_dependency_roots(crate_root: Path) -> set[Path]:
    """Find in-tree packages referenced by direct or inherited path dependencies."""
    document = _toml_document(read_text(crate_root / "Cargo.toml") or "")
    workspace_specs = _workspace_dependency_specifications(
        read_text(ROOT / "Cargo.toml")
    )
    roots: set[Path] = set()

    def collect(table: object) -> None:
        if not isinstance(table, dict):
            return
        for dependency, specification in table.items():
            if not isinstance(specification, dict):
                continue
            resolved = specification
            base = crate_root
            if specification.get("workspace") is True:
                resolved = workspace_specs.get(
                    _normalize_dependency_name(str(dependency)), {}
                )
                base = ROOT
            dependency_path = resolved.get("path")
            if not isinstance(dependency_path, str):
                continue
            candidate = (base / dependency_path).resolve()
            try:
                candidate.relative_to(ROOT.resolve())
            except ValueError:
                continue
            if (candidate / "Cargo.toml").is_file():
                roots.add(candidate)

    for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
        collect(document.get(table_name))
    targets = document.get("target", {})
    if isinstance(targets, dict):
        for target in targets.values():
            if not isinstance(target, dict):
                continue
            for table_name in ("dependencies", "dev-dependencies", "build-dependencies"):
                collect(target.get(table_name))
    return roots


def _workspace_production_roots() -> list[Path] | None:
    roots = _workspace_member_roots()
    if roots is None:
        return None
    discovered = set(roots)
    pending = list(roots)
    while pending:
        crate_root = pending.pop()
        for dependency_root in _path_dependency_roots(crate_root):
            if dependency_root not in discovered:
                discovered.add(dependency_root)
                pending.append(dependency_root)
    return sorted(discovered)


def _library_target_path(crate_root: Path) -> Path | None:
    manifest = _toml_document(read_text(crate_root / "Cargo.toml") or "")
    lib = manifest.get("lib")
    if isinstance(lib, dict) and isinstance(lib.get("path"), str):
        return crate_root / str(lib["path"])
    default = crate_root / "src" / "lib.rs"
    return default if default.is_file() else None


def production_lib_paths() -> list[Path]:
    roots = _workspace_production_roots()
    if roots is None:
        candidates = ROOT.glob("**/src/lib.rs")
    else:
        candidates = (_library_target_path(root) for root in roots)
    return sorted(
        source
        for source in candidates
        if source is not None
        and source.is_file()
        and not should_skip(source)
        and not is_test_source(source)
    )


_FN_DECL_PATTERN = re.compile(
    rf"^\s*(?:pub(?:\s*\([^)]*\))?\s+)?{_FN_QUALIFIERS}fn\s+(\w+)\s*\(",
    re.MULTILINE,
)


def _find_function_bodies(text: str) -> list[tuple[str, str]]:
    """Extract (name, body) for each top-level function in *text*."""
    results: list[tuple[str, str]] = []
    i = 0
    while i < len(text):
        match = _FN_DECL_PATTERN.search(text, i)
        if match is None:
            break
        name = match.group(1)
        j = match.end()
        brace_depth = 0
        in_string = False
        string_char = None
        start = 0
        while j < len(text):
            c = text[j]
            if in_string:
                if c == "\\" and j + 1 < len(text):
                    j += 2
                    continue
                if c == string_char:
                    in_string = False
                j += 1
                continue
            if c in "\"'":
                in_string = True
                string_char = c
                j += 1
                continue
            if c == "{":
                if brace_depth == 0:
                    start = j
                brace_depth += 1
            elif c == "}":
                brace_depth -= 1
                if brace_depth == 0:
                    body = text[start + 1 : j]
                    results.append((name, body))
                    i = j + 1
                    break
            j += 1
        else:
            break
    return results


def _normalize_dependency_name(name: str) -> str:
    return name.strip().lower().replace("_", "-")


def _toml_document(content: str) -> dict[str, object]:
    try:
        import tomllib

        document = tomllib.loads(content)
    except (tomllib.TOMLDecodeError, ValueError):
        return {}
    return document if isinstance(document, dict) else {}


def _workspace_dependency_specifications(
    content: str | None,
) -> dict[str, dict[str, object]]:
    if content is None:
        return {}
    document = _toml_document(content)
    specifications: dict[str, dict[str, object]] = {}

    def collect(table: object) -> None:
        if not isinstance(table, dict):
            return
        for dependency, specification in table.items():
            if isinstance(specification, dict):
                specifications[_normalize_dependency_name(str(dependency))] = specification
            else:
                specifications[_normalize_dependency_name(str(dependency))] = {}

    workspace = document.get("workspace")
    if isinstance(workspace, dict):
        collect(workspace.get("dependencies"))
    targets = document.get("target", {})
    if isinstance(targets, dict):
        for target in targets.values():
            if not isinstance(target, dict):
                continue
            target_workspace = target.get("workspace")
            if isinstance(target_workspace, dict):
                collect(target_workspace.get("dependencies"))
    return specifications
