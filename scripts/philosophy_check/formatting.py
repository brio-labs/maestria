"""Formatting rule family."""

from __future__ import annotations

from . import contract_tests, shared
from .shared import (
    _MOD_DECL_PATTERN,
    _find_function_bodies,
    _logical_line_count_scrubbed,
    _production_rust_files,
    _rust_syntax,
    _scrubbed_production,
    _scrubbed_source,
    is_test_source,
    logical_line_count,
    production_rust,
    read_text,
)
import datetime
import re

MAX_PRODUCTION_LINE_WIDTH = 100


MAX_PRODUCTION_LOGICAL_LINES = 400


MAX_MODULE_PHYSICAL_LINES = 900


MAX_FUNCTION_LOGICAL_LINES = 100


MODULE_SIZE_EXEMPTIONS: dict[str, str] = {
    "crates/core/maestria-core/src/manifest_codec.rs": "2027-03-31",
}


FUNCTION_SIZE_EXEMPTIONS: dict[str, dict[str, str]] = {
    "crates/kernel/maestria-domain/src/search_outcome/rerank.rs": {
        "validate": "2027-03-31",
    },
    "crates/core/maestria-core/src/manifest_encoding.rs": {
        "encode": "2027-03-31",
    },
    "crates/apps/maestria-daemon/src/search_executor.rs": {
        "assemble": "2027-03-31",
    },
}

MIXED_RESPONSIBILITY_EXEMPTIONS: dict[str, str] = {
    "crates/apps/maestria-daemon/src/search_executor.rs": "2027-03-31",
}
ADR_MODULE_EXEMPTIONS: dict[str, str] = {}

def scan_exemption_expiry(today: str | None = None) -> list[str]:
    current_text = today or datetime.date.today().isoformat()
    try:
        current = datetime.date.fromisoformat(current_text)
    except ValueError:
        return [f"exemption scan date {current_text!r} is not a ISO calendar date"]

    violations: list[str] = []
    exemptions: dict[str, str] = {
        **MODULE_SIZE_EXEMPTIONS,
        **ADR_MODULE_EXEMPTIONS,
        **MIXED_RESPONSIBILITY_EXEMPTIONS,
    }
    for path, items in FUNCTION_SIZE_EXEMPTIONS.items():
        for item, target_text in items.items():
            exemptions[f"{path}::{item}"] = target_text
    for path, target_text in sorted(exemptions.items()):
        try:
            target = datetime.date.fromisoformat(target_text)
        except ValueError:
            violations.append(
                f"{path} has malformed exemption expiry {target_text!r} "
                "(expected a YYYY-MM-DD calendar date)"
            )
            continue
        if current >= target:
            violations.append(
                f"{path} exemption expired at {target_text} "
                "(today is {today}); refactor or renew the ADR".format(
                    today=current.isoformat()
                )
            )
    return violations

def scan_module_sizes() -> list[str]:
    violations = []
    for source in _production_rust_files(skip_tests=False):
        rel_path = source.relative_to(shared.ROOT)
        rel = rel_path.as_posix()
        content = read_text(source)
        if content is None:
            continue
        if rel in MODULE_SIZE_EXEMPTIONS or rel in ADR_MODULE_EXEMPTIONS:
            continue
        logical_lines = _logical_line_count_scrubbed(_scrubbed_source(source))
        physical_lines = len(content.splitlines())
        if logical_lines > MAX_PRODUCTION_LOGICAL_LINES and not is_test_source(
            rel_path
        ):
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


def scan_cohesion() -> list[str]:
    """Emit cohesion / concept-density signals for large façade modules.

    Flags lib.rs files where the ratio of logical lines per declared module
    exceeds a heuristic threshold, indicating a single file may be carrying
    too many responsibilities.
    """
    violations = []
    for rel_path, declared_modules in contract_tests.RESPONSIBILITY_MAPS.items():
        source = shared.ROOT / rel_path
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


def scan_function_sizes() -> list[str]:
    """Flag production functions that exceed the logical-line budget."""
    violations: list[str] = []
    for source in _production_rust_files(skip_tests=True):
        rel_path = source.relative_to(shared.ROOT)
        rel = rel_path.as_posix()
        content = read_text(source)
        if content is None:
            continue
        exemptions = FUNCTION_SIZE_EXEMPTIONS.get(rel, {})
        scrubbed = _scrubbed_production(source)
        for name, body in _find_function_bodies(scrubbed):
            lines = _logical_line_count_scrubbed(body)
            if lines > MAX_FUNCTION_LOGICAL_LINES and name not in exemptions:
                violations.append(
                    f"{rel} function `{name}` has {lines} logical lines "
                    f"(limit {MAX_FUNCTION_LOGICAL_LINES})"
                )
    return violations


def _statement_terminator_count(line: str) -> int:
    """Count statement terminators outside grouping delimiters."""
    round_depth = 0
    square_depth = 0
    count = 0
    for token in line:
        if token == "(":
            round_depth += 1
        elif token == ")" and round_depth:
            round_depth -= 1
        elif token == "[":
            square_depth += 1
        elif token == "]" and square_depth:
            square_depth -= 1
        elif token == ";" and round_depth == 0 and square_depth == 0:
            count += 1
    return count


def scan_readability_style() -> list[str]:
    one_line_function = re.compile(r"(?m)\bfn\s+\w+\b[^{\n]*\{([^{}\n]*)\}")
    violations: list[str] = []
    for source in _production_rust_files(skip_tests=True):
        content = read_text(source)
        if content is None:
            continue
        production = production_rust(content)
        scrubbed = _rust_syntax(production)
        code_only = _rust_syntax(production, "\0")
        rel = source.relative_to(shared.ROOT).as_posix()
        for line_number, line in enumerate(code_only.splitlines(), 1):
            code_line = line.replace("\0", "")
            if code_line.strip() and len(code_line.rstrip()) > MAX_PRODUCTION_LINE_WIDTH:
                violations.append(
                    f"{rel}:{line_number} production code line is "
                    f"{len(code_line.rstrip())} characters "
                    f"(limit {MAX_PRODUCTION_LINE_WIDTH})"
                )
        for line_number, line in enumerate(scrubbed.splitlines(), 1):
            if line.strip() and _statement_terminator_count(line) > 1:
                violations.append(
                    f"{rel}:{line_number} has multiple production statements on one line"
                )
        for match in one_line_function.finditer(scrubbed):
            body = match.group(1).strip()
            if not body:
                continue
            line_number = scrubbed.count("\n", 0, match.start()) + 1
            violations.append(
                f"{rel}:{line_number} has a one-line production function body; "
                "expand it into a readable block"
            )
    return violations


def scan_mixed_responsibilities() -> list[str]:
    """Flag non-façade modules that declare many sub-modules and are large.

    Heuristic: a regular .rs file (not lib.rs / mod.rs / main.rs) that
    declares 3+ child modules *and* exceeds 300 logical lines is likely
    accumulating mixed responsibilities.
    """
    violations: list[str] = []
    mod_pattern = _MOD_DECL_PATTERN
    for source in _production_rust_files(skip_tests=True):
        if source.name in {"lib.rs", "mod.rs", "main.rs"}:
            continue
        rel_path = source.relative_to(shared.ROOT)
        rel = rel_path.as_posix()
        if rel in MIXED_RESPONSIBILITY_EXEMPTIONS:
            continue
        content = read_text(source)
        if content is None:
            continue
        production = production_rust(content)
        mods = mod_pattern.findall(production)
        logical = _logical_line_count_scrubbed(_scrubbed_production(source))
        if len(mods) >= 3 and logical > 300:
            violations.append(
                f"{rel} has {len(mods)} child modules and {logical} logical lines — "
                f"mixed-responsibility signal: extract to dedicated module or ADR"
            )
    return violations
