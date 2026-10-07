"""Shared comparison and measurement arithmetic for v2 evidence checks."""

from __future__ import annotations

import sys
from decimal import Decimal, InvalidOperation
from typing import Any

_artifact_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_artifacts"]
_SOURCE_ROLE_TO_FIELD = _artifact_helpers._SOURCE_ROLE_TO_FIELD
def _same_json(left: Any, right: Any) -> bool:
    if type(left) is not type(right):
        return False
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(_same_json(left[key], right[key]) for key in left)
    if isinstance(left, list):
        return len(left) == len(right) and all(_same_json(a, b) for a, b in zip(left, right))
    return left == right
def _decimal(value: Any) -> Decimal | None:
    if type(value) not in (int, float):
        return None
    try:
        result = Decimal(str(value))
    except InvalidOperation:
        return None
    return result if result.is_finite() else None
def _aggregate(values: list[Decimal], method: str) -> Decimal | None:
    if not values:
        return None
    if method == "sum":
        return sum(values, Decimal(0))
    if method in {"mean", "paired_mean_difference"}:
        return sum(values, Decimal(0)) / Decimal(len(values))
    ordered = sorted(values)
    if method == "minimum":
        return ordered[0]
    if method == "maximum":
        return ordered[-1]
    if method == "median":
        middle = len(ordered) // 2
        return ordered[middle] if len(ordered) % 2 else (
            ordered[middle - 1] + ordered[middle]
        ) / Decimal(2)
    if method == "p95":
        rank = (95 * len(ordered) + 99) // 100
        return ordered[rank - 1]
    return None
