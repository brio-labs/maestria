"""Read-only public views derived from verified v2 benchmark evidence."""

from __future__ import annotations

import re
import sys
from decimal import Decimal
from typing import Any

_common = sys.modules["_sillage_benchmark_run_evidence_v2_common"]
_measurements = sys.modules["_sillage_benchmark_run_evidence_v2_measurements"]
_decimal = _common._decimal
_aggregate = _common._aggregate
_denominator_count = _measurements._denominator_count

_VIEW_VERSION = 1
_REPORTABLE_EVIDENCE_ERRORS = {
    "campaign consumption exceeds the declared budget",
    "preparation wall time exceeds its frozen maximum",
    "verified resource measurement exceeds its frozen maximum",
}


def blocked_view(kind: str, diagnostic: str) -> dict[str, Any]:
    record_kind = {
        "references": "benchmark_reference_index_view",
        "claims": "benchmark_claim_view",
    }.get(kind, "benchmark_evidence_view")
    base = {
        "view_version": _VIEW_VERSION,
        "view_kind": kind,
        "record_kind": record_kind,
        "derived": True,
        "read_only": True,
    }
    if kind == "references":
        return {
            **base,
            "artifact_verification_complete": False,
            "artifacts": [],
            "references": [],
            "diagnostics": [diagnostic],
        }
    return {
        **base,
        "qualification": {
            "eligible": False,
            "status": "blocked",
            "claim_ids": [],
            "diagnostics": [diagnostic],
        },
        "measurements": [],
        "first_attempts": [],
        "attempts": [],
        "activities": [],
        "totals": None,
        "reference_index": blocked_view("references", diagnostic),
    }


def _artifact_status(
    index: int,
    descriptor: dict[str, Any],
    verified: dict[str, Any],
    artifact_errors: list[str],
    warnings: list[str],
) -> str:
    classification = descriptor["access"]["classification"]
    if classification == "unavailable":
        return "unavailable"
    if descriptor["artifact_id"] in verified:
        return "verified"
    prefix = f"artifact[{index}]"
    if any(message.startswith(prefix) for message in warnings):
        return "unavailable"
    if any(message.startswith(prefix) for message in artifact_errors):
        return "invalid"
    return "unverified"


def _safe_identifier(value: Any) -> Any:
    if not isinstance(value, str):
        return value
    if (
        re.search(r"[a-fA-F0-9]{32,128}", value)
        or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]{0,127}", value)
        or re.fullmatch(
            r"(?:gh[pousr]_|github_pat_|sk-)[A-Za-z0-9_-]{20,}",
            value,
            re.IGNORECASE,
        )
        or (value.startswith("eyJ") and value.count(".") == 2)
    ):
        return None
    return value


def _public_path(descriptor: dict[str, Any], status: str) -> str | None:
    path = descriptor["path"]
    parts = path.split("/")
    if (
        descriptor["access"]["classification"] == "public"
        and status == "verified"
        and re.fullmatch(r"[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", path)
        and all(part not in {".", ".."} for part in parts)
        and not any(re.search(r"[a-fA-F0-9]{32,128}", part) for part in parts)
    ):
        return path
    return None


def _reference_identity_matches(
    reference: dict[str, Any], descriptor: dict[str, Any], verified: dict[str, Any]
) -> bool:
    if reference["path"] != descriptor["path"]:
        return False
    reference_digest = reference["sha256"]
    if reference_digest is not None and (
        reference_digest.lower() != descriptor["sha256"].lower()
    ):
        return False
    artifact = verified.get(descriptor["artifact_id"])
    content = artifact.content if artifact is not None else None
    if reference["kind"] == "run":
        return (
            isinstance(content, dict)
            and reference["run_id"] == content.get("run_id")
            and reference["attempt_id"] == content.get("attempt_id")
        )
    if reference["run_id"] is not None or reference["attempt_id"] is not None:
        if not isinstance(content, dict):
            return False
        if reference["run_id"] is not None and (
            content.get("run_id") != reference["run_id"]
        ):
            return False
        if reference["attempt_id"] is not None and (
            content.get("attempt_id") != reference["attempt_id"]
        ):
            return False
    return True


def build_reference_index(
    payload: dict[str, Any],
    verified: dict[str, Any],
    artifact_errors: list[str],
    warnings: list[str],
    artifact_root_provided: bool,
) -> dict[str, Any]:
    descriptors = payload["artifacts"]
    artifacts_by_id: dict[str, tuple[int, dict[str, Any], str]] = {}
    artifact_rows = []
    for index, descriptor in enumerate(descriptors):
        status = _artifact_status(index, descriptor, verified, artifact_errors, warnings)
        artifacts_by_id[descriptor["artifact_id"]] = (index, descriptor, status)
        artifact_rows.append({
            "run_id": _safe_identifier(payload["run_id"]),
            "attempt_id": _safe_identifier(payload.get("attempt_id")),
            "artifact_id": _safe_identifier(descriptor["artifact_id"]),
            "role": _safe_identifier(descriptor["role"]),
            "path": _public_path(descriptor, status),
            "access": _safe_identifier(descriptor["access"]["classification"]),
            "status": status,
        })

    reference_rows = []
    for reference in payload["prior_evidence_refs"]:
        match = artifacts_by_id.get(reference["artifact_id"])
        descriptor = match[1] if match is not None else None
        status = match[2] if match is not None else "unverified"
        if descriptor is not None and status == "verified":
            if not _reference_identity_matches(reference, descriptor, verified):
                status = "invalid"
        reference_rows.append({
            "kind": _safe_identifier(reference["kind"]),
            "manifest_id": _safe_identifier(reference.get("manifest_id")),
            "run_id": _safe_identifier(reference["run_id"]),
            "attempt_id": _safe_identifier(reference["attempt_id"]),
            "artifact_id": _safe_identifier(reference["artifact_id"]),
            "relation": _safe_identifier(reference["relation"]),
            "provides_measurements_for_this_run": reference[
                "provides_measurements_for_this_run"
            ],
            "path": _public_path(descriptor, status) if descriptor is not None else None,
            "access": (
                _safe_identifier(descriptor["access"]["classification"])
                if descriptor is not None else None
            ),
            "status": status,
        })

    return {
        "view_version": _VIEW_VERSION,
        "view_kind": "references",
        "record_kind": "benchmark_reference_index_view",
        "derived": True,
        "read_only": True,
        "run_id": _safe_identifier(payload["run_id"]),
        "attempt_id": _safe_identifier(payload.get("attempt_id")),
        "artifact_verification_complete": (
            artifact_root_provided and bool(descriptors) and len(verified) == len(descriptors)
        ),
        "artifacts": artifact_rows,
        "references": reference_rows,
    }


def _diagnostic_for_eligibility(reason: str) -> str:
    codes = {
        "example record": "example_record",
        "run is not complete": "run_not_complete",
        "run is not confirmatory": "experiment_not_confirmatory",
        "protocol is not frozen and digest-bound": "protocol_not_frozen",
        "campaign or attempt identity is missing": "run_identity_missing",
        "start/terminal receipts are incomplete or unbound": "lifecycle_receipts_incomplete",
        "all six product and OS cache boundaries must be explicit": "cache_boundaries_incomplete",
        "confirmatory execution requires a clean source tree": "source_tree_not_clean",
        "source commit identity is incomplete": "source_identity_incomplete",
        "build or environment identity is incomplete": "environment_identity_incomplete",
        "execution provenance is incomplete": "execution_provenance_incomplete",
        "hardware identity is incomplete": "hardware_identity_incomplete",
        "runtime or seed identity is incomplete": "runtime_identity_incomplete",
        "source identity is incomplete": "source_identity_incomplete",
        "comparison systems are incomplete": "comparison_systems_incomplete",
        "budget scope or currency is incomplete": "budget_identity_incomplete",
        "trace, access, or retention policy is incomplete": "access_policy_incomplete",
        "trace origin does not have the required affirmative consent": "consent_not_established",
        "reviewable execution provenance is incomplete": "execution_provenance_incomplete",
        "manifest does not declare a supported claim": "claim_not_declared_eligible",
        "measurement-bearing evidence reference is unavailable or unverified": "measurement_reference_unverified",
        "required measurements are unavailable": "required_measurements_unavailable",
    }
    return codes.get(reason, "confirmatory_eligibility_not_established")


def _measurement_rows(
    payload: dict[str, Any],
    context: dict[str, Any],
    first: dict[str, dict[str, Any]],
    consumptions: list[dict[str, Any]],
    content_valid: bool,
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Decimal]]]:
    requirements = context["requirements"]
    observations = {
        item["name"]: item for item in context["records"]["observations"]["measurements"]
    }
    declared = {item["name"]: item for item in payload["measurements"]}
    planned = context["planned"]
    raw_by_name: dict[str, dict[str, Decimal]] = {}
    raw_by_metric_system: dict[tuple[str, str], dict[str, Decimal]] = {}
    invalid_names: set[str] = set()

    for requirement in requirements:
        if (
            requirement["scope"] != "system"
            or requirement["aggregation"] == "paired_mean_difference"
            or declared[requirement["name"]]["status"] != "measured"
        ):
            continue
        selected = [slot for slot in planned if slot["system_id"] == requirement["system_id"]]
        rows = observations.get(requirement["name"], {}).get("observations", [])
        by_slot = {row["slot_id"]: row for row in rows}
        if len(by_slot) != len(rows) or set(by_slot) != {slot["slot_id"] for slot in selected}:
            invalid_names.add(requirement["name"])
            continue
        values: dict[str, Decimal] = {}
        for slot in selected:
            row, attempt = by_slot[slot["slot_id"]], first.get(slot["slot_id"])
            value = _decimal(row["value"])
            if (
                attempt is None
                or row["run_id"] != attempt["run_id"]
                or row["attempt_id"] != attempt["attempt_id"]
                or row["state"] != attempt["state"]
                or value is None
                or (
                    requirement["metric"] == "ndcg_at_10"
                    and not Decimal(0) <= value <= Decimal(1)
                )
                or (requirement["role"] == "constraint" and value < 0)
            ):
                invalid_names.add(requirement["name"])
                break
            values[slot["slot_id"]] = value
        if requirement["name"] not in invalid_names:
            raw_by_name[requirement["name"]] = values
            raw_by_metric_system[(requirement["metric"], requirement["system_id"])] = values

    for requirement in requirements:
        if (
            requirement["scope"] != "system"
            or requirement["aggregation"] != "paired_mean_difference"
            or declared[requirement["name"]]["status"] != "measured"
        ):
            continue
        selected = [slot for slot in planned if slot["system_id"] == requirement["system_id"]]
        rows = observations.get(requirement["name"], {}).get("observations", [])
        by_slot = {row["slot_id"]: row for row in rows}
        values: dict[str, Decimal] = {}
        for slot in selected:
            baseline_slot = next((item for item in planned if (
                item["system_id"] == requirement["comparison_system_id"]
                and item["case_id"] == slot["case_id"]
                and item["repetition"] == slot["repetition"]
            )), None)
            candidate_values = raw_by_metric_system.get(
                ("ndcg_at_10", requirement["system_id"]), {}
            )
            baseline_values = raw_by_metric_system.get(
                ("ndcg_at_10", requirement["comparison_system_id"]), {}
            )
            candidate = candidate_values.get(slot["slot_id"])
            baseline = baseline_values.get(baseline_slot["slot_id"]) if baseline_slot else None
            row, attempt = by_slot.get(slot["slot_id"]), first.get(slot["slot_id"])
            delta = candidate - baseline if candidate is not None and baseline is not None else None
            if (
                row is None or attempt is None or delta is None
                or row["run_id"] != attempt["run_id"]
                or row["attempt_id"] != attempt["attempt_id"]
                or row["state"] != attempt["state"]
                or _decimal(row["value"]) != delta
            ):
                invalid_names.add(requirement["name"])
                break
            values[slot["slot_id"]] = delta
        if requirement["name"] not in invalid_names:
            raw_by_name[requirement["name"]] = values

    rows_out: list[dict[str, Any]] = []
    for requirement in requirements:
        name = requirement["name"]
        measure = declared[name]
        status = measure["status"]
        reason = {
            "unavailable": "measurement_unavailable",
            "not_run": "measurement_not_run",
            "not_applicable": "measurement_not_applicable",
            "invalid": "measurement_invalid",
            "estimated": "estimated_measurement_not_recomputed",
        }.get(status, "derived_from_verified_artifacts")
        value: Decimal | None = None
        denominator: dict[str, Any] | None = None
        selected = [
            slot for slot in planned
            if requirement["scope"] == "system"
            and slot["system_id"] == requirement["system_id"]
        ]
        if requirement["scope"] == "system":
            denominator = {
                "kind": _safe_identifier(requirement["denominator_kind"]),
                "count": _denominator_count(requirement, selected, context),
            }
        elif requirement["metric"] == "preparation_wall_ms":
            campaign_values = [
                _decimal(item["wall_time_ms"])
                for item in consumptions if item["phase"] == "preparation"
            ]
            if all(item is not None for item in campaign_values):
                denominator = {
                    "kind": _safe_identifier(requirement["denominator_kind"]),
                    "count": len(campaign_values),
                }
        if status == "measured" and content_valid and name not in invalid_names:
            if requirement["scope"] == "campaign":
                if requirement["metric"] == "preparation_wall_ms":
                    value = _aggregate(campaign_values, requirement["aggregation"])
            elif requirement["aggregation"] == "paired_mean_difference":
                values = raw_by_name.get(name)
                if values is not None:
                    value = _aggregate(
                        [values[slot["slot_id"]] for slot in selected],
                        requirement["aggregation"],
                    )
            else:
                values = raw_by_name.get(name)
                if values is not None:
                    value = _aggregate(
                        [values[slot["slot_id"]] for slot in selected],
                        requirement["aggregation"],
                    )
        if status == "measured" and not content_valid:
            status = "unverified"
            reason = "evidence_validation_failed"
        elif status == "measured" and value is None:
            status = "unverified"
            reason = "source_measurement_unverified"
        rows_out.append({
            "name": _safe_identifier(name),
            "role": _safe_identifier(requirement["role"]),
            "value": float(value) if value is not None else None,
            "unit": _safe_identifier(requirement["unit"]),
            "status": status,
            "reason": reason,
            "method": _safe_identifier(requirement["aggregation"]),
            "denominator": denominator,
        })
    return rows_out, raw_by_name


def _unverified_measurement_rows(
    context: dict[str, Any], reason: str = "evidence_validation_failed"
) -> list[dict[str, Any]]:
    return [{
        "name": _safe_identifier(requirement["name"]),
        "role": _safe_identifier(requirement["role"]),
        "value": None,
        "unit": _safe_identifier(requirement["unit"]),
        "status": "unverified",
        "reason": reason,
        "method": _safe_identifier(requirement["aggregation"]),
        "denominator": None,
    } for requirement in context["requirements"]]


def _declared_unverified_measurements(
    payload: dict[str, Any], reason: str
) -> list[dict[str, Any]]:
    return [{
        "name": _safe_identifier(item["name"]),
        "role": _safe_identifier(item["role"]),
        "value": None,
        "unit": None,
        "status": "unverified",
        "reason": reason,
        "method": None,
        "denominator": None,
    } for item in payload["measurements"]]


def _attempt_rows(
    context: dict[str, Any],
    first: dict[str, dict[str, Any]],
    measurements: dict[str, dict[str, Decimal]],
    content_valid: bool,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]], list[dict[str, Any]], dict[str, Any]]:
    ledger = context["records"]["attempt_ledger"]
    planned = context["planned"]
    first_rows = []
    for slot in planned:
        attempt = first.get(slot["slot_id"])
        if attempt is None:
            continue
        observed = []
        if content_valid:
            for requirement in context["requirements"]:
                value = measurements.get(requirement["name"], {}).get(slot["slot_id"])
                if value is not None:
                    observed.append({
                        "name": _safe_identifier(requirement["name"]),
                        "value": float(value),
                        "unit": _safe_identifier(requirement["unit"]),
                    })
        first_rows.append({
            "ordinal": slot["ordinal"],
            "system_id": _safe_identifier(slot["system_id"]),
            "repetition": slot["repetition"],
            "state": attempt["state"],
            "attempt_number": 1,
            "measurements": observed,
        })

    attempts = [{
        "ordinal": item["ordinal"],
        "system_id": _safe_identifier(item["system_id"]),
        "repetition": item["repetition"],
        "attempt_number": item["attempt_number"],
        "state": item["state"],
        "compute_seconds": item["compute_seconds"],
        "cost_amount": item["cost_amount"],
    } for item in ledger["attempts"]]
    activities = [{
        "phase": _safe_identifier(item["phase"]),
        "attempt_number": item["attempt_number"],
        "state": item["state"],
        "compute_seconds": item["compute_seconds"],
        "cost_amount": item["cost_amount"],
        "wall_time_ms": item["wall_time_ms"],
    } for item in ledger["consumptions"]]
    all_rows = [*ledger["attempts"], *ledger["consumptions"]]
    compute = sum((Decimal(str(item["compute_seconds"])) for item in all_rows), Decimal(0))
    cost = sum((Decimal(str(item["cost_amount"])) for item in all_rows), Decimal(0))
    totals = {
        "currency": _safe_identifier(ledger["currency"]),
        "compute_seconds": float(compute),
        "cost_amount": float(cost),
    }
    return first_rows, attempts, activities, totals




def content_is_invalid(source: dict[str, Any] | None) -> bool:
    if source is None or source.get("context") is None:
        return True
    phases = source.get("phase_errors", {})
    if any(phases.get(name) for name in (
        "roles", "protocol", "bindings", "corrections", "unknown"
    )):
        return True
    return any(
        error not in _REPORTABLE_EVIDENCE_ERRORS
        for error in source.get("content_errors", [])
    )


def build_claim_view(
    payload: dict[str, Any],
    source: dict[str, Any] | None,
    eligible: bool,
    eligibility_reasons: list[str],
    blocked_diagnostics: list[str] | None = None,
    reference_index: dict[str, Any] | None = None,
) -> tuple[dict[str, Any], int]:
    diagnostics = list(blocked_diagnostics or [])
    context = source.get("context") if source is not None else None
    content_errors = source.get("content_errors", []) if source is not None else []
    fatal_content = content_is_invalid(source)
    for error in content_errors:
        if error in _REPORTABLE_EVIDENCE_ERRORS:
            diagnostics.append(
                "budget_exceeded"
                if error == "campaign consumption exceeds the declared budget"
                else "measurement_constraint_exceeded"
            )

    if source is None:
        diagnostics.append("artifact_verification_incomplete")
        measurements = _declared_unverified_measurements(
            payload, "artifact_verification_incomplete"
        )
        first_attempts: list[dict[str, Any]] = []
        attempts: list[dict[str, Any]] = []
        activities: list[dict[str, Any]] = []
        totals = None
        state = "blocked"
    elif context is None:
        diagnostics.append("evidence_content_invalid")
        measurements = _declared_unverified_measurements(
            payload, "evidence_validation_failed"
        )
        first_attempts = []
        attempts = []
        activities = []
        totals = None
        state = "blocked"
    elif fatal_content:
        return blocked_view("claims", "evidence_content_invalid"), 1
    elif "measurement_reference_unverified" in diagnostics:
        measurements = _unverified_measurement_rows(
            context, "measurement_reference_unverified"
        )
        first_attempts, attempts, activities, totals = _attempt_rows(
            context, source.get("first", {}), {}, content_valid=False
        )
        state = "blocked"
    else:
        measurements, per_slot_values = _measurement_rows(
            payload, context, source.get("first", {}), source.get("consumptions", []),
            content_valid=True,
        )
        first_attempts, attempts, activities, totals = _attempt_rows(
            context, source.get("first", {}), per_slot_values, content_valid=True
        )
        state = "qualified" if eligible else "ineligible"

    for reason in eligibility_reasons:
        diagnostics.append(_diagnostic_for_eligibility(reason))
    if not eligible and not diagnostics:
        diagnostics.append("confirmatory_eligibility_not_established")
    diagnostics = list(dict.fromkeys(diagnostics))
    qualification_status = "qualified" if state == "qualified" and eligible else state
    view = {
        "view_version": _VIEW_VERSION,
        "view_kind": "claims",
        "record_kind": "benchmark_claim_view",
        "derived": True,
        "read_only": True,
        "run_id": _safe_identifier(payload["run_id"]),
        "attempt_id": _safe_identifier(payload.get("attempt_id")),
        "run_state": _safe_identifier(payload["state"]),
        "qualification": {
            "eligible": bool(eligible and qualification_status == "qualified"),
            "status": qualification_status,
            "claim_ids": [
                _safe_identifier(item) for item in payload["claim_eligibility"]["claim_ids"]
            ],
            "diagnostics": diagnostics,
        },
        "measurements": measurements,
        "first_attempts": first_attempts,
        "attempts": attempts,
        "activities": activities,
        "totals": totals,
        "reference_index": reference_index,
    }
    return view, 0 if view["qualification"]["eligible"] else 1
