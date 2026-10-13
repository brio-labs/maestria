"""Shared paths, source iteration, and Rust text helpers."""

from __future__ import annotations

import os
import re
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


THIS_SCRIPT = ROOT / "scripts" / "philosophy-check.py"

# The checker's own package carries pattern literals (marker words, secret
# shapes) that must never trip the scans they define.
PACKAGE_DIR = Path(__file__).resolve().parent


def _is_checker_source(path: Path) -> bool:
    resolved = path.resolve()
    return resolved == THIS_SCRIPT or resolved.parent == PACKAGE_DIR


DOMAIN_ROOT = ROOT / "crates" / "kernel" / "sillage-domain"


DOMAIN_SRC = DOMAIN_ROOT / "src"


DOMAIN_MANIFEST = DOMAIN_ROOT / "Cargo.toml"


KERNEL_ROOTS = (
    ROOT / "crates" / "kernel" / "sillage-domain",
    ROOT / "crates" / "kernel" / "sillage-governance",
    ROOT / "crates" / "kernel" / "sillage-ports",
)


SCAN_EXTS = {".rs", ".toml", ".py", ".yml", ".yaml", ".md"}


SKIP_DIRS = {".git", "target", "node_modules", "dist", ".direnv", ".venv"}


# Files exempted from the doctrine scan (e.g. vendored or legacy artifacts).
SKIP_FILES = set()

# Immutable corpus snapshots are source data, not active Rust compilation units.
# General source and secret scans still inspect these historical bytes.
FROZEN_SOURCE_ROOT = ROOT / "tests" / "frozen-corpus-snapshots"


def should_skip(path: Path) -> bool:
    rel = path.relative_to(ROOT)
    rel_parts = set(rel.parts)
    return (
        _is_checker_source(path)
        or rel.as_posix() in SKIP_FILES
        or bool(rel_parts.intersection(SKIP_DIRS))
    )


def _external_patch_roots() -> set[Path]:
    """Keep upstream Cargo patches outside first-party Rust doctrine checks.

    Both an explicit path patch and a workspace exclusion are required.
    General source and secret scans still inspect these dependency files.
    """
    document = _toml_document(read_text(ROOT / "Cargo.toml") or "")
    workspace = document.get("workspace", {})
    patches = document.get("patch", {})
    if not isinstance(workspace, dict) or not isinstance(patches, dict):
        return set()
    patterns = workspace.get("exclude", [])
    if not isinstance(patterns, list):
        return set()
    excluded = {
        match.resolve()
        for pattern in patterns
        if isinstance(pattern, str)
        for match in ROOT.glob(pattern)
    }
    roots: set[Path] = set()
    for replacements in patches.values():
        if not isinstance(replacements, dict):
            continue
        for specification in replacements.values():
            if not isinstance(specification, dict):
                continue
            relative = specification.get("path")
            if not isinstance(relative, str):
                continue
            candidate = (ROOT / relative).resolve()
            if candidate == ROOT or not candidate.is_relative_to(ROOT):
                continue
            if any(candidate.is_relative_to(exclusion) for exclusion in excluded):
                roots.add(candidate)
    return roots


def _rust_source_paths() -> Iterator[Path]:
    # Prune only directories whose descendants should_skip already rejects.
    for directory, names, filenames in os.walk(ROOT):
        names[:] = [name for name in names if name not in SKIP_DIRS]
        for name in filenames:
            if name.endswith(".rs"):
                yield Path(directory) / name


def _production_rust_files(*, skip_tests: bool = True, sorted_: bool = False) -> Iterator[Path]:
    external_patches = _external_patch_roots()
    test_context = _module_test_context() if skip_tests else {}
    iterator = sorted(_rust_source_paths()) if sorted_ else _rust_source_paths()
    for path in iterator:
        if should_skip(path):
            continue
        if any(path.is_relative_to(root) for root in external_patches):
            continue
        if path.is_relative_to(FROZEN_SOURCE_ROOT):
            continue
        if skip_tests and _is_test_only_rust_path(path, test_context):
            continue
        yield path


def _kernel_rust_files(*, skip_tests: bool = True, sorted_: bool = False) -> Iterator[Path]:
    test_context = _module_test_context() if skip_tests else {}
    for kernel_root in KERNEL_ROOTS:
        files = (kernel_root / "src").rglob("*.rs")
        if sorted_:
            files = sorted(files)
        for path in files:
            if skip_tests and _is_test_only_rust_path(path, test_context):
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
    """Like [`_scrubbed_source`], but masks `#[cfg(test)]` items so test
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


_CFG_ATTRIBUTE = re.compile(
    r"#\s*\[\s*cfg\s*\((?P<expression>[^\]]*)\)\s*\]",
    re.DOTALL,
)


def _split_cfg_arguments(expression: str) -> list[str]:
    arguments: list[str] = []
    depth = 0
    start = 0
    for index, character in enumerate(expression):
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
            if depth < 0:
                return []
        elif character == "," and depth == 0:
            arguments.append(expression[start:index].strip())
            start = index + 1
    if depth != 0:
        return []
    arguments.append(expression[start:].strip())
    return arguments


def _cfg_expression_requires_test(expression: str) -> bool:
    expression = expression.strip()
    if expression == "test":
        return True
    condition = re.fullmatch(r"(all|any|not)\s*\((.*)\)", expression, re.DOTALL)
    if condition is None:
        return False
    operator, arguments_text = condition.groups()
    arguments = _split_cfg_arguments(arguments_text)
    if not arguments or any(not argument for argument in arguments):
        return False
    if operator == "all":
        return any(_cfg_expression_requires_test(argument) for argument in arguments)
    if operator == "any":
        return all(_cfg_expression_requires_test(argument) for argument in arguments)
    return False


def _cfg_attribute_requires_test(attribute: str) -> bool:
    condition = re.fullmatch(r"\s*cfg\s*\((.*)\)\s*", attribute, re.DOTALL)
    return (
        condition is not None
        and _cfg_expression_requires_test(condition.group(1))
    )


def _cfg_test_item_extents(text: str) -> list[tuple[int, int]]:
    """Extents of cfg-gated items and inner scopes requiring test context.

    Offsets are located in syntax-scrubbed text so occurrences inside
    comments or string literals never trigger removal.
    """
    scrubbed = _rust_syntax(text)
    leading_attributes, _ = _attributes_at(
        text, scrubbed, 0, len(scrubbed)
    )
    if any(
        inner and _cfg_attribute_requires_test(clean)
        for clean, _raw, inner in leading_attributes
    ):
        return [(0, len(text))]
    extents: list[tuple[int, int]] = []
    search_from = 0
    while match := _CFG_ATTRIBUTE.search(scrubbed, search_from):
        if not _cfg_expression_requires_test(match.group("expression")):
            search_from = match.end()
            continue
        start = match.start()
        end = _gated_item_end(scrubbed, start)
        extents.append((start, end))
        search_from = end
    if "#![" in scrubbed:
        for declaration in _rust_module_declarations(
            text,
            include_local=False,
            scrubbed=scrubbed,
        ):
            if not declaration.inline or any(
                start <= declaration.start < end for start, end in extents
            ):
                continue
            module = _RUST_MODULE_ITEM.match(
                scrubbed, declaration.start, declaration.end
            )
            if module is None:
                continue
            opening = _skip_rust_whitespace(
                scrubbed, module.end(), declaration.end
            )
            if opening >= declaration.end or scrubbed[opening] != "{":
                continue
            closing = _matching_delimiter(scrubbed, opening, "{", "}")
            if closing is None or closing >= declaration.end:
                continue
            inner_attributes, _ = _attributes_at(
                text, scrubbed, opening + 1, closing
            )
            if _inner_scope_requires_test(inner_attributes):
                extents.append(
                    (declaration.item_prefix_start, declaration.end)
                )
    return extents


def production_rust(text: str) -> str:
    """Production portion of Rust source with test-only scopes blanked.

    Test-only imports, modules, and helper functions are excluded while
    source positions and line breaks after them remain stable.
    """
    extents = _cfg_test_item_extents(text)
    if not extents:
        return text
    chars = list(text)
    for start, end in extents:
        _blank_non_newlines(chars, start, end)
    return "".join(chars)


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


@dataclass(frozen=True)
class RustModuleDeclaration:
    """A module item parsed from syntax-scrubbed Rust source."""

    name: str
    start: int
    end: int
    parent_modules: tuple[str, ...]
    inline: bool
    cfg_test: bool
    test_only: bool
    path: str | None
    path_attribute: bool
    path_supported: bool
    item_prefix_start: int

_RUST_PUBLIC_KEYWORD = re.compile(r"pub\b")
_RUST_MODULE_ITEM = re.compile(r"mod\s+([A-Za-z_][A-Za-z0-9_]*)\b")
_RUST_GENERIC_BRACED_ITEM = re.compile(
    r"(?:(?:async|const|unsafe|extern|default)\s+)*"
    r"(?:fn|impl|trait|struct|enum|union)\b"
)
_RUST_MACRO_BRACED_ITEM = re.compile(
    r"(?:\bmacro_rules\s*!\s*[A-Za-z_]\w*|\bmacro\s+[A-Za-z_]\w*|"
    r"(?:\$crate::)?(?:[A-Za-z_]\w*::)*[A-Za-z_]\w*\s*!)\s*$"
)
_RUST_INCLUDE_MACRO = re.compile(r"\binclude\s*!\s*\(")

def _skip_rust_whitespace(text: str, index: int, end: int) -> int:
    while index < end and text[index].isspace():
        index += 1
    return index


def _attributes_at(
    source: str,
    scrubbed: str,
    index: int,
    end: int,
) -> tuple[list[tuple[str, str, bool]], int]:
    attributes: list[tuple[str, str, bool]] = []
    cursor = index
    while True:
        cursor = _skip_rust_whitespace(scrubbed, cursor, end)
        if cursor + 1 >= end or scrubbed[cursor] != "#":
            return attributes, cursor
        if scrubbed.startswith("#![", cursor):
            opening = cursor + 2
            inner = True
        elif scrubbed.startswith("#[", cursor):
            opening = cursor + 1
            inner = False
        else:
            return attributes, cursor
        closing = _matching_delimiter(scrubbed, opening, "[", "]")
        if closing is None or closing >= end:
            return attributes, cursor
        attributes.append(
            (
                scrubbed[opening + 1 : closing],
                source[opening + 1 : closing],
                inner,
            )
        )
        cursor = closing + 1


def _rust_string_literal_value(value: str) -> str | None:
    literal = re.fullmatch(r'\s*"([^"\\]*)"\s*', value, re.DOTALL)
    if literal is not None:
        return literal.group(1)
    raw_literal = re.fullmatch(
        r'\s*r(?P<hashes>#{0,255})"(?P<value>.*?)"(?P=hashes)\s*',
        value,
        re.DOTALL,
    )
    return raw_literal.group("value") if raw_literal is not None else None

def _inner_scope_requires_test(
    attributes: list[tuple[str, str, bool]],
) -> bool:
    return any(
        inner and _cfg_attribute_requires_test(clean)
        for clean, _raw, inner in attributes
    )


def _module_attribute_data(
    attributes: list[tuple[str, str, bool]],
) -> tuple[bool, str | None, bool, bool]:
    cfg_test = any(
        not inner and _cfg_attribute_requires_test(clean)
        for clean, _raw, inner in attributes
    )
    path_attributes = [
        (clean, raw)
        for clean, raw, inner in attributes
        if not inner and re.search(r"\bpath\s*=", clean)
    ]
    if not path_attributes:
        return cfg_test, None, False, False
    if len(path_attributes) != 1:
        return cfg_test, None, True, False

    clean, raw = path_attributes[0]
    if re.fullmatch(r"\s*path\s*=\s*", clean) is None:
        path_matches = list(re.finditer(r"\bpath\s*=", clean))
        if len(path_matches) != 1:
            return cfg_test, None, True, False
        raw_value = raw[path_matches[0].end() :]
        conditional_literal = re.match(
            r'\s*"([^"\\]*)"', raw_value, re.DOTALL
        )
        if conditional_literal is not None:
            return cfg_test, conditional_literal.group(1), True, False
        conditional_raw_literal = re.match(
            r'\s*r(?P<hashes>#{0,255})"(?P<value>.*?)"(?P=hashes)',
            raw_value,
            re.DOTALL,
        )
        if conditional_raw_literal is not None:
            return (
                cfg_test,
                conditional_raw_literal.group("value"),
                True,
                False,
            )
        return cfg_test, None, True, False
    literal = re.fullmatch(r'\s*path\s*=\s*"([^"\\]*)"\s*', raw, re.DOTALL)
    if literal is not None:
        return cfg_test, literal.group(1), True, True
    raw_literal = re.fullmatch(
        r'\s*path\s*=\s*r(?P<hashes>#{0,255})"(?P<value>.*?)"(?P=hashes)\s*',
        raw,
        re.DOTALL,
    )
    if raw_literal is None:
        return cfg_test, None, True, False
    return cfg_test, raw_literal.group("value"), True, True


def _skip_rust_item(text: str, index: int, end: int) -> int:
    depths = {"(": 0, "[": 0, "{": 0}
    closing = {")": "(", "]": "[", "}": "{"}
    cursor = index
    while cursor < end:
        token = text[cursor]
        if token == ";" and not any(depths.values()):
            return cursor + 1
        if token == "{" and depths["("] == depths["["] == depths["{"] == 0:
            closing_index = _matching_delimiter(text, cursor, "{", "}")
            return end if closing_index is None else closing_index + 1
        if token in depths:
            depths[token] += 1
        elif token in closing:
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
        cursor += 1
    return end


def _rust_braced_item_body(
    text: str,
    index: int,
    end: int,
) -> tuple[int, int] | None:
    """Find the next non-macro braced scope in syntax-scrubbed Rust."""
    generic_item = _RUST_GENERIC_BRACED_ITEM.match(text, index, end) is not None
    depths = {"(": 0, "[": 0}
    closing = {")": "(", "]": "["}
    if generic_item:
        depths["<"] = 0
        closing[">"] = "<"
    for cursor in range(index, end):
        token = text[cursor]
        if token == "{" and not any(depths.values()):
            if _RUST_MACRO_BRACED_ITEM.search(text[index:cursor].rstrip()):
                return None
            body_end = _matching_delimiter(text, cursor, "{", "}")
            if body_end is None or body_end >= end:
                return None
            return cursor, body_end
        if token == ";" and not any(depths.values()):
            return None
        if token in depths:
            depths[token] += 1
        elif token in closing:
            if token == ">" and cursor > index and text[cursor - 1] == "-":
                continue
            opener = closing[token]
            depths[opener] = max(0, depths[opener] - 1)
    return None


def _module_item_start(text: str, index: int, end: int) -> int:
    visibility = _RUST_PUBLIC_KEYWORD.match(text, index, end)
    if visibility is None:
        return index
    cursor = _skip_rust_whitespace(text, visibility.end(), end)
    if cursor < end and text[cursor] == "(":
        closing = _matching_delimiter(text, cursor, "(", ")")
        if closing is None or closing >= end:
            return end
        cursor = closing + 1
    return _skip_rust_whitespace(text, cursor, end)

def _module_item_prefix_start(text: str, index: int, end: int) -> int:
    cursor = _skip_rust_whitespace(text, index, end)
    while text.startswith("#![", cursor):
        opening = cursor + 2
        closing = _matching_delimiter(text, opening, "[", "]")
        if closing is None or closing >= end:
            return cursor
        cursor = _skip_rust_whitespace(text, closing + 1, end)
    return cursor

def _rust_module_declarations(
    text: str,
    *,
    include_local: bool,
    scrubbed: str | None = None,
) -> tuple[RustModuleDeclaration, ...]:
    """Parse module items, optionally including block-local declarations."""
    scrubbed = _rust_syntax(text) if scrubbed is None else scrubbed
    declarations: list[RustModuleDeclaration] = []

    def parse_scope(
        scope_start: int,
        scope_end: int,
        parent_modules: tuple[str, ...],
        inherited_test_only: bool,
    ) -> None:
        cursor = scope_start
        while cursor < scope_end:
            scope_item_start = cursor
            attributes, item_start = _attributes_at(
                text, scrubbed, cursor, scope_end
            )
            if _inner_scope_requires_test(attributes):
                inherited_test_only = True
            attributes = [
                attribute for attribute in attributes if not attribute[2]
            ]
            item_start = _module_item_start(scrubbed, item_start, scope_end)
            module = _RUST_MODULE_ITEM.match(
                scrubbed, item_start, scope_end
            )
            if module is None:
                if not include_local:
                    cursor = _skip_rust_item(scrubbed, item_start, scope_end)
                    continue
                item_body = _rust_braced_item_body(
                    scrubbed, item_start, scope_end
                )
                if item_body is None:
                    cursor = _skip_rust_item(scrubbed, item_start, scope_end)
                    continue
                body_start, body_end = item_body
                body_test_only = inherited_test_only or _module_attribute_data(
                    attributes
                )[0]
                parse_scope(
                    body_start + 1,
                    body_end,
                    parent_modules,
                    body_test_only,
                )
                cursor = body_end + 1
                continue

            item_prefix_start = _module_item_prefix_start(
                scrubbed, scope_item_start, scope_end
            )
            name = module.group(1)
            delimiter = _skip_rust_whitespace(
                scrubbed, module.end(), scope_end
            )
            cfg_test, path, path_attribute, path_supported = (
                _module_attribute_data(attributes)
            )
            test_only = inherited_test_only or cfg_test
            if delimiter < scope_end and scrubbed[delimiter] == ";":
                module_end = delimiter + 1
                declarations.append(
                    RustModuleDeclaration(
                        name,
                        module.start(),
                        module_end,
                        parent_modules,
                        False,
                        cfg_test,
                        test_only,
                        path,
                        path_attribute,
                        path_supported,
                        item_prefix_start,
                    )
                )
                cursor = module_end
                continue
            if delimiter < scope_end and scrubbed[delimiter] == "{":
                closing = _matching_delimiter(
                    scrubbed, delimiter, "{", "}"
                )
                if closing is None or closing >= scope_end:
                    cursor = scope_end
                    continue
                module_end = closing + 1
                declarations.append(
                    RustModuleDeclaration(
                        name,
                        module.start(),
                        module_end,
                        parent_modules,
                        True,
                        cfg_test,
                        test_only,
                        path,
                        path_attribute,
                        path_supported,
                        item_prefix_start,
                    )
                )
                parse_scope(
                    delimiter + 1,
                    closing,
                    parent_modules + (name,),
                    test_only,
                )
                cursor = module_end
                continue
            cursor = _skip_rust_item(scrubbed, item_start, scope_end)

    parse_scope(0, len(scrubbed), (), False)
    return tuple(declarations)


def rust_module_declarations(text: str) -> tuple[RustModuleDeclaration, ...]:
    """Return module items with their actual cfg and path attribute context."""
    return _rust_module_declarations(text, include_local=False)


def _rust_include_paths(
    text: str,
    scrubbed: str | None = None,
) -> tuple[tuple[str, ...], bool]:
    """Return literal include paths and an opaque production-reference flag."""
    scrubbed = _rust_syntax(text) if scrubbed is None else scrubbed
    paths: list[str] = []
    opaque = False
    production_source: str | None = None
    for include in _RUST_INCLUDE_MACRO.finditer(scrubbed):
        opening = include.end() - 1
        closing = _matching_delimiter(scrubbed, opening, "(", ")")
        path = (
            None
            if closing is None
            else _rust_string_literal_value(text[opening + 1 : closing])
        )
        if path is None:
            if production_source is None:
                production_source = production_rust(text)
            if production_source[include.start()] == text[include.start()]:
                opaque = True
        else:
            paths.append(path)
    return tuple(paths), opaque


def _top_level_module_declarations(text: str) -> set[str]:
    """Return root-level external declarations not gated by `cfg(test)`."""
    return {
        declaration.name
        for declaration in rust_module_declarations(text)
        if not declaration.parent_modules
        and not declaration.inline
        and not declaration.test_only
    }

_MODULE_CONTEXT_ROOT: Path | None = None
_MODULE_CONTEXT: dict[Path, frozenset[bool]] = {}
_MODULE_CONTEXT_SIGNATURE: tuple[
    tuple[Path, int, int, int, int, int], ...
] | None = None


def _crate_module_roots(crate_root: Path) -> set[tuple[Path, bool]]:
    """Return Cargo target roots paired with their test-only context."""
    document = _toml_document(read_text(crate_root / "Cargo.toml") or "")
    package = document.get("package")
    if not isinstance(package, dict):
        return set()
    project_root = ROOT.resolve()
    roots: set[tuple[Path, bool]] = set()

    def add(path: object, test_only: bool) -> None:
        if not isinstance(path, str):
            return
        candidate = Path(path)
        if not candidate.is_absolute():
            candidate = crate_root / candidate
        try:
            candidate = candidate.resolve()
        except (OSError, RuntimeError):
            return
        if candidate.is_file() and candidate.is_relative_to(project_root):
            roots.add((candidate, test_only))

    library = document.get("lib")
    library_path = (
        library.get("path")
        if isinstance(library, dict) and isinstance(library.get("path"), str)
        else "src/lib.rs"
    )
    add(library_path, False)

    def target_entries(name: str) -> list[dict[str, object]]:
        targets = document.get(name)
        if isinstance(targets, dict):
            return [targets]
        if isinstance(targets, list):
            return [target for target in targets if isinstance(target, dict)]
        return []

    def conventional_target_path(
        target_name: str,
        target: dict[str, object],
    ) -> Path | None:
        target_name_value = target.get("name")
        if not isinstance(target_name_value, str) or not target_name_value:
            return None
        if (
            target_name == "bin"
            and target_name_value == package.get("name")
        ):
            main_binary = crate_root / "src" / "main.rs"
            if main_binary.is_file():
                return main_binary
        directory_name = {
            "bin": "src/bin",
            "example": "examples",
            "test": "tests",
            "bench": "benches",
        }.get(target_name)
        if directory_name is None:
            return None
        flat_target = (
            crate_root / directory_name / f"{target_name_value}.rs"
        )
        if flat_target.is_file():
            return flat_target
        nested_target = (
            crate_root / directory_name / target_name_value / "main.rs"
        )
        return nested_target if nested_target.is_file() else None

    for target_name, test_only in (
        ("bin", False),
        ("example", False),
        ("test", True),
        ("bench", False),
    ):
        for target in target_entries(target_name):
            target_path = target.get("path")
            if target_path is None:
                inferred_path = conventional_target_path(target_name, target)
                if inferred_path is None:
                    continue
                target_path = inferred_path.as_posix()
            add(target_path, test_only)

    if package.get("autobins") is not False:
        add("src/main.rs", False)
    build_script = package.get("build", True)
    if build_script is not False:
        add(
            build_script if isinstance(build_script, str) else "build.rs",
            False,
        )
    for directory_name, automatic_target_flag in (
        ("src/bin", "autobins"),
        ("examples", "autoexamples"),
        ("benches", "autobenches"),
    ):
        if package.get(automatic_target_flag) is False:
            continue
        directory = crate_root / directory_name
        if not directory.is_dir():
            continue
        for source in directory.glob("*.rs"):
            add(source.as_posix(), False)
        for source in directory.glob("*/main.rs"):
            add(source.as_posix(), False)

    tests_directory = crate_root / "tests"
    if package.get("autotests") is not False and tests_directory.is_dir():
        for source in tests_directory.glob("*.rs"):
            add(source.as_posix(), True)
        for source in tests_directory.glob("*/main.rs"):
            add(source.as_posix(), True)
    return roots

def _rust_module_reachability(
    project_root: Path,
) -> dict[Path, frozenset[bool]]:
    reachability: dict[Path, set[bool]] = {}
    visited: set[tuple[Path, bool, bool, Path]] = set()
    crate_roots = _workspace_production_roots()
    package_files: dict[Path, set[Path]] = {}
    opaque_packages: set[Path] = set()

    def visit(
        path: Path,
        test_only: bool,
        crate_root_file: bool,
        package_root: Path,
    ) -> None:
        try:
            source_path = path.resolve()
        except (OSError, RuntimeError):
            return
        if (
            not source_path.is_file()
            or not source_path.is_relative_to(project_root)
        ):
            return
        source = read_text(source_path)
        scrubbed = _rust_syntax(source) if source is not None else None
        effective_test_only = test_only
        if source is not None and scrubbed is not None:
            leading_attributes, _ = _attributes_at(
                source, scrubbed, 0, len(scrubbed)
            )
            if _inner_scope_requires_test(leading_attributes):
                effective_test_only = True
        reachability.setdefault(source_path, set()).add(effective_test_only)
        package_files.setdefault(package_root, set()).add(source_path)
        state = (
            source_path,
            effective_test_only,
            crate_root_file,
            package_root,
        )
        if state in visited:
            return
        visited.add(state)

        if source is None or scrubbed is None:
            return
        include_paths, opaque_include = _rust_include_paths(source, scrubbed)
        # Only opaque references in shipping sources taint their package.
        if opaque_include and not effective_test_only:
            opaque_packages.add(package_root)
        module_directory = (
            source_path.parent
            if crate_root_file or source_path.name == "mod.rs"
            else source_path.with_suffix("")
        )
        declarations = _rust_module_declarations(
            source,
            include_local=True,
            scrubbed=scrubbed,
        )
        inline_modules: dict[
            tuple[str, ...], list[RustModuleDeclaration]
        ] = {}
        for declaration in declarations:
            if declaration.inline:
                prefix = declaration.parent_modules + (declaration.name,)
                inline_modules.setdefault(prefix, []).append(declaration)
        for declaration in declarations:
            if declaration.inline:
                continue
            child_test_only = effective_test_only or declaration.test_only
            parent_directories = {module_directory}
            prefix: tuple[str, ...] = ()
            for parent in declaration.parent_modules:
                prefix += (parent,)
                inline_parent = next(
                    (
                        candidate
                        for candidate in inline_modules.get(prefix, ())
                        if candidate.start
                        <= declaration.start
                        < candidate.end
                    ),
                    None,
                )
                next_directories: set[Path] = set()
                for parent_directory in parent_directories:
                    if inline_parent is None or not inline_parent.path_attribute:
                        next_directories.add(parent_directory / parent)
                        continue
                    if inline_parent.path is None:
                        if not child_test_only:
                            opaque_packages.add(package_root)
                        next_directories.add(parent_directory / parent)
                        continue
                    inline_path = Path(inline_parent.path)
                    if not inline_path.is_absolute():
                        base_directory = (
                            source_path.parent
                            if not inline_parent.parent_modules
                            else parent_directory
                        )
                        inline_path = base_directory / inline_path
                    next_directories.add(inline_path)
                    # Conditional paths leave the default module directory
                    # active for the other feature configurations.
                    if not inline_parent.path_supported:
                        next_directories.add(parent_directory / parent)
                parent_directories = next_directories
            if not parent_directories:
                continue
            if (
                declaration.path_attribute
                and not declaration.path_supported
                and child_test_only
            ):
                continue
            if (
                declaration.path_attribute
                and declaration.path is None
                and declaration.path_supported
            ):
                continue
            candidates: list[Path] = []
            for parent_directory in parent_directories:
                default_candidates = [
                    parent_directory / f"{declaration.name}.rs",
                    parent_directory / declaration.name / "mod.rs",
                ]
                if not declaration.path_attribute:
                    candidates.extend(default_candidates)
                    continue
                if declaration.path is None:
                    if not child_test_only:
                        opaque_packages.add(package_root)
                    candidates.extend(default_candidates)
                    continue
                target = Path(declaration.path)
                if not target.is_absolute():
                    # Top-level path attributes resolve beside the source file;
                    # nested ones resolve from their inline module directory.
                    base_directory = (
                        source_path.parent
                        if not declaration.parent_modules
                        else parent_directory
                    )
                    target = base_directory / target
                candidates.append(target)
                # A conditional `path` selects an alternate to the default.
                if not declaration.path_supported:
                    candidates.extend(default_candidates)

            existing: set[Path] = set()
            for candidate in candidates:
                try:
                    resolved = candidate.resolve()
                except (OSError, RuntimeError):
                    continue
                if (
                    resolved.is_file()
                    and resolved.is_relative_to(project_root)
                ):
                    existing.add(resolved)
            if len(existing) == 1:
                visit(next(iter(existing)), child_test_only, False, package_root)
            elif len(existing) > 1:
                # Ambiguous module layouts are invalid Rust; do not let either
                # candidate inherit a test-only exemption.
                for candidate in existing:
                    visit(candidate, False, False, package_root)

        for include_path in include_paths:
            target = Path(include_path)
            if not target.is_absolute():
                target = source_path.parent / target
            visit(target, False, False, package_root)

    standalone_tree = (
        crate_roots is None and not (project_root / "Cargo.toml").exists()
    )
    if standalone_tree:
        for source in _rust_source_paths():
            if (
                source.name in {"lib.rs", "main.rs"}
                and source.parent.name == "src"
            ):
                visit(source, False, True, project_root)
    elif crate_roots is not None:
        for crate_root in crate_roots:
            for target, test_only in _crate_module_roots(crate_root):
                visit(target, test_only, True, crate_root)


    # Opaque shipping references may target any Rust source reached by their
    # package; never downgrade unrelated workspace packages.
    for package_root in opaque_packages:
        for source in package_files.get(package_root, ()):
            contexts = reachability[source]
            if True in contexts:
                contexts.add(False)
    return {
        source: frozenset(contexts)
        for source, contexts in reachability.items()
    }


def _module_source_signature(
    project_root: Path,
) -> tuple[tuple[Path, int, int, int, int, int], ...]:
    sources: list[tuple[Path, int, int, int, int, int]] = []
    for directory, names, filenames in os.walk(project_root):
        names[:] = [name for name in names if name not in SKIP_DIRS]
        for name in filenames:
            if name != "Cargo.toml" and not name.endswith(".rs"):
                continue
            path = Path(directory) / name
            try:
                stat = path.stat()
            except OSError:
                continue
            sources.append(
                (
                    path,
                    stat.st_dev,
                    stat.st_ino,
                    stat.st_mtime_ns,
                    stat.st_ctime_ns,
                    stat.st_size,
                )
            )
    return tuple(sorted(sources))


def _module_test_context() -> dict[Path, frozenset[bool]]:
    """Cache classification per root and source signature, never across edits."""
    global _MODULE_CONTEXT_ROOT, _MODULE_CONTEXT, _MODULE_CONTEXT_SIGNATURE
    project_root = ROOT.resolve()
    signature = _module_source_signature(project_root)
    if (
        _MODULE_CONTEXT_ROOT != project_root
        or _MODULE_CONTEXT_SIGNATURE != signature
    ):
        context = _rust_module_reachability(project_root)
        _MODULE_CONTEXT = context
        _MODULE_CONTEXT_ROOT = project_root
        _MODULE_CONTEXT_SIGNATURE = signature
    return _MODULE_CONTEXT


def _is_test_only_rust_path(
    path: Path,
    context: dict[Path, frozenset[bool]],
) -> bool:
    try:
        source = path if path.is_absolute() else ROOT / path
        source = source.resolve()
    except (OSError, RuntimeError):
        return False
    reachability = context.get(source)
    return (
        reachability is not None
        and True in reachability
        and False not in reachability
    )


def is_test_source(path: Path) -> bool:
    """True only for Rust sources reachable through test targets or `cfg(test)`."""
    path = Path(path)
    if path.suffix.lower() == ".rs":
        return _is_test_only_rust_path(path, _module_test_context())
    return (
        "tests" in path.parts
        or any(segment.endswith("_tests") for segment in path.parts)
        or path.stem in {"tests", "contract_tests", "test_support"}
        or path.stem.endswith("_tests")
        or path.stem.endswith("_test")
        or path.stem.startswith("tests_")
        or (path.suffix == ".py" and path.stem.startswith("test_"))
    )



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
            if c == "'":
                lifetime_end = j + 1
                if lifetime_end < len(text) and (
                    text[lifetime_end].isalpha() or text[lifetime_end] == "_"
                ):
                    lifetime_end += 1
                    while lifetime_end < len(text) and (
                        text[lifetime_end].isalnum() or text[lifetime_end] == "_"
                    ):
                        lifetime_end += 1
                    if lifetime_end >= len(text) or text[lifetime_end] != "'":
                        j = lifetime_end
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