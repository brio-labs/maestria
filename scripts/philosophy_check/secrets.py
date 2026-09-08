"""Secrets rule family."""

from __future__ import annotations

from . import shared
from .shared import (
    SCAN_EXTS,
    _iter_scannable,
    is_test_source,
    production_rust,
    read_text,
)
import re

# Security: hardcoded secret material. The vocabulary mirrors the governance
# privacy scanner (`scan_secrets` in maestria-governance) so the repository
# guardrail and the domain's secret policy classify the same shapes.
_SECRET_PRIVATE_KEY_PATTERN = re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")


_SECRET_ACCESS_TOKEN_PATTERN = re.compile(
    r"\b(?:AKIA[0-9A-Z]{10,}|"
    r"ghp_[A-Za-z0-9]{20,}|"
    r"github_pat_[A-Za-z0-9_]{20,}|"
    r"xox[bp]-[A-Za-z0-9-]{10,}|"
    r"sk_live_[A-Za-z0-9]{10,})\b"
)


_SECRET_ASSIGNMENT_KEYS = ("password", "passwd", "api_key", "apikey", "secret", "token")


_SECRET_PLACEHOLDER_VALUES = {"xxx", "changeme", "example", "placeholder"}


def _secret_assignment_on_line(line: str, is_code: bool) -> bool:
    """True when a line assigns a non-placeholder value to a credential key.

    Mirrors the governance privacy scanner's `contains_credential_assignment`
    for `password`/`api_key`/`secret`/`token` keys. Code files additionally
    require the value to be a literal or bare word so Rust fields
    (`token: String,`) and expressions (`token = text[idx]`) do not trip it;
    CI templates (`${{ secrets.X }}`) and `<placeholder>` values never do.
    """
    assignment = line.strip()
    if assignment.startswith("export"):
        remainder = assignment[6:].lstrip()
        if remainder:
            assignment = remainder
    name, separator, value = assignment.partition("=")
    if not separator:
        name, separator, value = assignment.partition(":")
    if not separator:
        return False
    normalized_name = name.strip().strip('"\'{}').strip().lower()
    if normalized_name not in _SECRET_ASSIGNMENT_KEYS:
        return False
    candidate = value.strip()
    if is_code:
        # A struct field, type, or expression is not a credential literal.
        if not (
            candidate.startswith('"')
            or candidate.startswith("'")
            or re.fullmatch(r"[A-Za-z0-9_\-\.]+", candidate)
        ):
            return False
    else:
        if not candidate or candidate.startswith("${"):
            return False
        candidate = candidate.strip('"\' ,}')
        if not candidate:
            return False
    if candidate.startswith("<") and candidate.endswith(">"):
        return False
    return candidate.lower().strip(".") not in _SECRET_PLACEHOLDER_VALUES


def scan_hardcoded_secrets() -> list[str]:
    """Security: no hardcoded secret material in non-test files.

    Detects private-key blocks, access-token shapes (AWS/GitHub/Slack/Stripe
    live keys), and credential assignments with the same vocabulary as the
    governance privacy scanner. Test sources are exempt: fake credentials are
    legitimate fixtures there (the governance scanner tests exercise them).
    Rust files are cfg(test)-stripped so inline test modules do not exempt
    production lines.
    """
    violations = []
    for candidate in _iter_scannable(shared.ROOT):
        if shared._is_checker_source(candidate):
            continue
        if candidate.suffix.lower() not in SCAN_EXTS:
            continue
        if is_test_source(candidate):
            continue
        content = read_text(candidate)
        if content is None:
            continue
        if candidate.suffix.lower() == ".rs":
            content = production_rust(content)
        is_code = candidate.suffix.lower() in {".rs", ".py"}
        for line_number, line in enumerate(content.splitlines(), 1):
            trimmed = line.strip()
            if _SECRET_PRIVATE_KEY_PATTERN.search(trimmed):
                kind = "hardcoded private key material"
            elif _SECRET_ACCESS_TOKEN_PATTERN.search(trimmed):
                kind = "a hardcoded access-token pattern"
            elif _secret_assignment_on_line(trimmed, is_code):
                kind = "a hardcoded credential assignment"
            else:
                continue
            violations.append(
                f"{candidate.relative_to(shared.ROOT)}:{line_number} contains {kind}"
            )
    return violations
