"""Frozen measurement and observation validation for v2 evidence."""

from __future__ import annotations

import sys
from decimal import Decimal
from typing import Any

_artifact_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_artifacts"]
_common_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_common"]
VerifiedArtifact = _artifact_helpers.VerifiedArtifact
_decimal = _common_helpers._decimal
_aggregate = _common_helpers._aggregate

def _observation_context(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    context: dict[str, Any],
    errors: list[str],
) -> tuple[
    dict[str, dict[str, Any]],
    dict[str, dict[str, Any]],
    list[dict[str, Any]],
    list[dict[str, Any]],
] | None:
    required, records = context["required"], context["records"]
    evidence = records["observations"]
    ledger_id = required["attempt_ledger"]["artifact_id"]
    if (
        evidence["campaign_id"] != context["campaign_id"]
        or evidence["protocol_digest"] != context["protocol_digest"]
        or evidence["attempt_ledger_digest"] != verified[ledger_id].digest
    ):
        errors.append("observations do not bind the verified campaign ledger")
    requirements = context["requirements"]
    manifest = {item["name"]: item for item in payload["measurements"]}
    system_requirements = [
        item for item in requirements
        if item["scope"] == "system" and manifest[item["name"]]["status"] == "measured"
    ]
    observed_rows = evidence["measurements"]
    if (
        len(observed_rows) != len(system_requirements)
        or len({item["name"] for item in observed_rows}) != len(observed_rows)
    ):
        errors.append("observations contain duplicate or omitted system measurement identities")
        return None
    observed = {item["name"]: item for item in observed_rows}
    paired = [
        item for item in system_requirements
        if item["aggregation"] == "paired_mean_difference"
    ]
    return observed, manifest, system_requirements, paired


def _measure_input(
    requirement: dict[str, Any],
    manifest: dict[str, dict[str, Any]],
    observed: dict[str, dict[str, Any]],
    observation_artifact_id: str,
    errors: list[str],
    paired: bool = False,
) -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]] | None:
    measure = manifest[requirement["name"]]
    if measure["status"] != "measured":
        return None
    item = observed.get(requirement["name"])
    denominator = measure.get("denominator")
    if (
        item is None
        or item["interval"] != measure["interval"]
        or measure["role"] != requirement["role"]
        or measure["unit"] != requirement["unit"]
        or measure["method"] != requirement["aggregation"]
        or measure["observations_artifact"] != observation_artifact_id
        or not isinstance(denominator, dict)
    ):
        message = (
            "paired primary measure is not bound to its frozen method and observations"
            if paired else "required measure is not bound to frozen method and observations"
        )
        errors.append(message)
        return None
    return measure, item, denominator


def _system_measurement_values(
    requirement: dict[str, Any],
    manifest: dict[str, dict[str, Any]],
    observed: dict[str, dict[str, Any]],
    observation_artifact_id: str,
    context: dict[str, Any],
    first: dict[str, dict[str, Any]],
    errors: list[str],
) -> dict[str, Decimal]:
    bound = _measure_input(
        requirement, manifest, observed, observation_artifact_id, errors
    )
    if bound is None:
        return {}
    measure, item, denominator = bound
    selected = [
        slot for slot in context["planned"]
        if slot["system_id"] == requirement["system_id"]
    ]
    obs_rows = item["observations"]
    by_slot = {row["slot_id"]: row for row in obs_rows}
    if (
        len(obs_rows) != len(selected)
        or len(by_slot) != len(obs_rows)
        or set(by_slot) != {slot["slot_id"] for slot in selected}
    ):
        errors.append("observations omit or add planned first-attempt slots")
        return {}
    values: list[Decimal] = []
    values_by_slot: dict[str, Decimal] = {}
    for slot in selected:
        row, attempt = by_slot[slot["slot_id"]], first[slot["slot_id"]]
        value = _decimal(row["value"])
        if (
            row["run_id"] != attempt["run_id"]
            or row["attempt_id"] != attempt["attempt_id"]
            or row["state"] != attempt["state"]
            or value is None
            or (
                requirement["metric"] == "ndcg_at_10"
                and not Decimal(0) <= value <= Decimal(1)
            )
            or (requirement["role"] == "constraint" and value < 0)
        ):
            errors.append("observation does not bind a first-attempt outcome")
            break
        values.append(value)
        values_by_slot[slot["slot_id"]] = value
    aggregate = _aggregate(values, requirement["aggregation"])
    count = _denominator_count(requirement, selected, context)
    if (
        aggregate is None
        or _decimal(measure["value"]) != aggregate
        or denominator["kind"] != requirement["denominator_kind"]
        or denominator["count"] != count
    ):
        errors.append("measurement or denominator does not match verified outcomes")
    if requirement["role"] == "constraint" and aggregate is not None:
        if aggregate > Decimal(str(requirement["max_value"])):
            errors.append("verified resource measurement exceeds its frozen maximum")
    return values_by_slot


def _paired_measurement(
    requirement: dict[str, Any],
    manifest: dict[str, dict[str, Any]],
    observed: dict[str, dict[str, Any]],
    observation_artifact_id: str,
    context: dict[str, Any],
    first: dict[str, dict[str, Any]],
    raw_values: dict[str, dict[str, dict[str, Decimal]]],
    plan_by_key: dict[tuple[str, str, int], dict[str, Any]],
    errors: list[str],
) -> None:
    bound = _measure_input(
        requirement, manifest, observed, observation_artifact_id, errors, paired=True
    )
    if bound is None:
        return
    measure, item, denominator = bound
    selected = [
        slot for slot in context["planned"]
        if slot["system_id"] == requirement["system_id"]
    ]
    observations = item["observations"]
    by_slot = {row["slot_id"]: row for row in observations}
    if (
        len(observations) != len(selected)
        or len(by_slot) != len(observations)
        or set(by_slot) != {slot["slot_id"] for slot in selected}
    ):
        errors.append("paired observations omit or add candidate first-attempt slots")
        return
    candidate_values = raw_values.get("ndcg_at_10", {}).get(
        requirement["system_id"], {}
    )
    baseline_values = raw_values.get("ndcg_at_10", {}).get(
        requirement["comparison_system_id"], {}
    )
    values: list[Decimal] = []
    for slot in selected:
        row, attempt = by_slot[slot["slot_id"]], first[slot["slot_id"]]
        baseline_slot = plan_by_key.get((
            requirement["comparison_system_id"], slot["case_id"], slot["repetition"]
        ))
        baseline = baseline_values.get(baseline_slot["slot_id"]) if baseline_slot else None
        candidate = candidate_values.get(slot["slot_id"])
        difference = candidate - baseline if candidate is not None and baseline is not None else None
        value = _decimal(row["value"])
        if (
            row["run_id"] != attempt["run_id"]
            or row["attempt_id"] != attempt["attempt_id"]
            or row["state"] != attempt["state"]
            or value is None
            or not Decimal(-1) <= value <= Decimal(1)
            or difference is None
            or value != difference
        ):
            errors.append("paired difference does not bind matched baseline and candidate outcomes")
            break
        values.append(value)
    aggregate = _aggregate(values, "paired_mean_difference")
    count = _denominator_count(requirement, selected, context)
    if (
        aggregate is None
        or _decimal(measure["value"]) != aggregate
        or denominator["kind"] != requirement["denominator_kind"]
        or denominator["count"] != count
    ):
        errors.append("paired primary estimate or denominator does not match matched outcomes")


def _campaign_measurements(
    requirements: list[dict[str, Any]],
    manifest: dict[str, dict[str, Any]],
    consumptions: list[dict[str, Any]],
    errors: list[str],
) -> None:
    for requirement in requirements:
        if requirement["scope"] != "campaign":
            continue
        measure = manifest[requirement["name"]]
        if measure["status"] != "measured":
            continue
        preparation = [
            _decimal(item["wall_time_ms"])
            for item in consumptions
            if item["phase"] == "preparation"
        ]
        values = [value for value in preparation if value is not None]
        aggregate = _aggregate(values, requirement["aggregation"])
        denominator = measure.get("denominator")
        if (
            measure["role"] != requirement["role"]
            or measure["unit"] != requirement["unit"]
            or measure["method"] != requirement["aggregation"]
            or measure["observations_artifact"] is not None
            or measure["interval"] is not None
            or not isinstance(denominator, dict)
            or aggregate is None
            or _decimal(measure["value"]) != aggregate
            or denominator["kind"] != "campaign_activities"
            or denominator["count"] != len(values)
        ):
            errors.append("preparation wall-time measure does not match campaign activity records")
        elif aggregate > Decimal(str(requirement["max_value"])):
            errors.append("preparation wall time exceeds its frozen maximum")

def _observation_evidence_errors(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    context: dict[str, Any],
    first: dict[str, dict[str, Any]],
    consumptions: list[dict[str, Any]],
    errors: list[str],
) -> None:
    view = _observation_context(payload, verified, context, errors)
    if view is None:
        return
    observed, manifest, system_requirements, paired_requirements = view
    observation_id = context["required"]["observations"]["artifact_id"]
    raw_values: dict[str, dict[str, dict[str, Decimal]]] = {}
    for requirement in system_requirements:
        if requirement["aggregation"] == "paired_mean_difference":
            continue
        raw_values.setdefault(requirement["metric"], {})[
            requirement["system_id"]
        ] = _system_measurement_values(
            requirement, manifest, observed, observation_id, context, first, errors
        )
    plan_by_key = {
        (slot["system_id"], slot["case_id"], slot["repetition"]): slot
        for slot in context["planned"]
    }
    for requirement in paired_requirements:
        _paired_measurement(
            requirement, manifest, observed, observation_id, context, first,
            raw_values, plan_by_key, errors,
        )
    _campaign_measurements(context["requirements"], manifest, consumptions, errors)


def _denominator_count(
    requirement: dict[str, Any],
    selected: list[dict[str, Any]],
    context: dict[str, Any],
) -> int:
    kind = requirement["denominator_kind"]
    if kind == "independent_needs":
        return len({slot["case_id"] for slot in selected})
    if kind == "independent_clusters":
        return len({context["clusters"][slot["case_id"]] for slot in selected})
    return len({slot["repetition"] for slot in selected})
