"""Type invariants rule family."""

from __future__ import annotations

from . import shared
from .shared import (
    _PUBLIC_FUNCTION_PATTERN,
    _function_parameters,
    _kernel_rust_files,
    _named_struct_fields,
    _scrubbed_production,
    _scrubbed_source,
    read_text,
)
import re

PRIMITIVE_ID_TYPES = {"String", "u8", "u16", "u32", "u64", "u128", "usize"}


STRINGLY_STATE_FIELD_NAMES = {
    "category",
    "class",
    "decision",
    "kind",
    "mode",
    "outcome",
    "phase",
    "state",
    "status",
    "strategy",
    "variant",
}


BOOLEAN_STATE_OPPOSITES = {
    "approved": "denied",
    "closed": "open",
    "completed": "pending",
    "disabled": "enabled",
    "failed": "passed",
    "failure": "success",
    "found": "missing",
    "invalid": "valid",
    "rejected": "accepted",
}


STATE_OPTIONAL_PAYLOADS = {
    "approved": {"approved_at", "approved_by"},
    "closed": {"closed_at", "close_reason"},
    "completed": {"completed_at", "completion_result"},
    "denied": {"denial_reason", "denied_at", "denied_by"},
    "failed": {"error", "error_message", "failed_at", "failure_reason"},
    "open": {"opened_at", "opened_by"},
    "rejected": {"rejected_at", "rejected_by", "rejection_reason"},
}


BOOLEAN_STATE_PREFIXES = ("is_", "was_")


def _state_name(field_name: str) -> str:
    for prefix in BOOLEAN_STATE_PREFIXES:
        if field_name.startswith(prefix):
            return field_name.removeprefix(prefix)
    return field_name


def _primitive_identity_groups(
    values: list[tuple[str, str]],
) -> dict[str, list[str]]:
    groups: dict[str, list[str]] = {}
    for name, value_type in values:
        if (
            name.endswith("_id")
            and not name.startswith(("max_", "min_"))
            and value_type in PRIMITIVE_ID_TYPES
        ):
            groups.setdefault(value_type, []).append(name)
    return {value_type: names for value_type, names in groups.items() if len(names) > 1}


def _struct_invariant_violations(struct_name: str, fields: list[tuple[str, str]], rel_path: Path) -> list[str]:
    violations = []
    bool_states = {_state_name(name) for name, value_type in fields if value_type == "bool"}
    for left, right in BOOLEAN_STATE_OPPOSITES.items():
        if left in bool_states and right in bool_states:
            violations.append(
                f"{rel_path} struct `{struct_name}` represents opposite states `{left}` and `{right}` as booleans; use an enum"
            )
    optional_fields = {name for name, value_type in fields if value_type.startswith("Option<")}
    for state, payload_names in STATE_OPTIONAL_PAYLOADS.items():
        correlated = sorted(optional_fields & payload_names)
        if state in bool_states and correlated:
            joined = "`, `".join(correlated)
            violations.append(
                f"{rel_path} struct `{struct_name}` coordinates boolean state `{state}` with optional payload `{joined}`; put the payload on an enum variant"
            )
    for name, value_type in fields:
        if name in STRINGLY_STATE_FIELD_NAMES and value_type in {"String", "&str", "&'static str"}:
            violations.append(
                f"{rel_path} struct `{struct_name}` represents state field `{name}` as `{value_type}`; use an enum or validated domain type"
            )
    for value_type, names in _primitive_identity_groups(fields).items():
        joined = "`, `".join(names)
        violations.append(
            f"{rel_path} struct `{struct_name}` has swappable primitive identities `{joined}` of type `{value_type}`; use distinct ID types"
        )
    return violations


def _function_invariant_violations(
    function_name: str, parameters: list[tuple[str, str]], public_functions: set[str], rel_path: Path
) -> list[str]:
    violations = []
    bool_states = {_state_name(name) for name, value_type in parameters if value_type == "bool"}
    for left, right in BOOLEAN_STATE_OPPOSITES.items():
        if left in bool_states and right in bool_states:
            violations.append(
                f"{rel_path} function `{function_name}` accepts opposite states `{left}` and `{right}` as booleans; accept an enum"
            )
    optional_parameters = {name for name, value_type in parameters if value_type.startswith("Option<")}
    for state, payload_names in STATE_OPTIONAL_PAYLOADS.items():
        correlated = sorted(optional_parameters & payload_names)
        if state in bool_states and correlated:
            joined = "`, `".join(correlated)
            violations.append(
                f"{rel_path} function `{function_name}` coordinates boolean state `{state}` with optional payload `{joined}`; accept an enum carrying the payload"
            )
    for name, value_type in parameters:
        if (
            function_name in public_functions
            and name in STRINGLY_STATE_FIELD_NAMES
            and value_type in {"String", "&str", "&'static str"}
        ):
            violations.append(
                f"{rel_path} function `{function_name}` accepts state parameter `{name}` as `{value_type}`; accept an enum or validated domain type"
            )
    for value_type, names in _primitive_identity_groups(parameters).items():
        joined = "`, `".join(names)
        violations.append(
            f"{rel_path} function `{function_name}` accepts swappable primitive identities `{joined}` of type `{value_type}`; use distinct ID types"
        )
    return violations


def scan_type_invariant_modeling() -> list[str]:
    """Reject high-confidence representations of invalid kernel states.

    This deliberately avoids blanket bans on booleans, optional values, and
    primitives. It catches only mechanically defensible cases; Rule 56 keeps
    correlated payloads and constructor visibility under architectural review.
    """
    violations = []
    for source in _kernel_rust_files(skip_tests=True):
        content = read_text(source)
        if content is None:
            continue
        rel_path = source.relative_to(shared.ROOT)
        production = _scrubbed_production(source)
        public_functions = set(_PUBLIC_FUNCTION_PATTERN.findall(production))
        for struct_name, fields in _named_struct_fields(production):
            violations.extend(_struct_invariant_violations(struct_name, fields, rel_path))
        for function_name, parameters in _function_parameters(production):
            violations.extend(
                _function_invariant_violations(function_name, parameters, public_functions, rel_path)
            )
    return violations


_UNNAMED_JSON_PATTERN = re.compile(r"\bserde_json::(?:Value|json!)")


def scan_domain_untyped_json() -> list[str]:
    """Rule 31: no untyped `serde_json::Value` holes in domain sources when
    the shape is known. DTO conversion at adapter boundaries is typed; a
    `serde_json::Value` or `json!` literal in the kernel is a hole."""
    violations = []
    for source in shared.DOMAIN_SRC.rglob("*.rs"):
        production = _scrubbed_source(source)
        if not production:
            continue
        if _UNNAMED_JSON_PATTERN.search(production):
            violations.append(
                f"{source.relative_to(shared.ROOT)} uses untyped serde_json::Value in domain source"
            )
    return violations
