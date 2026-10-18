"""Frozen protocol, source, and system bindings for v2 evidence."""

from __future__ import annotations

import sys
from typing import Any

_artifact_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_artifacts"]
_common_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_common"]
VerifiedArtifact = _artifact_helpers.VerifiedArtifact
_schema_errors = _artifact_helpers._schema_errors
_timestamp = _artifact_helpers._timestamp
_same_json = _common_helpers._same_json
_decimal = _common_helpers._decimal
_SOURCE_ROLE_TO_FIELD = _common_helpers._SOURCE_ROLE_TO_FIELD
def _content_role_index(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    schema: dict[str, Any],
    errors: list[str],
) -> dict[str, list[dict[str, Any]]]:
    definitions = {
        "protocol": "protocolEvidence",
        "execution": "executionEvidence",
        "declared_budget": "declaredBudgetEvidence",
        "attempt_ledger": "attemptLedgerEvidence",
        "access_record": "accessRecordEvidence",
        "observations": "observationsEvidence",
        "case_list": "caseListEvidence",
        "start_receipt": "startReceiptEvidence",
        "terminal_receipt": "terminalReceiptEvidence",
        "system_configuration": "systemConfigurationEvidence",
        "campaign_consumption": "campaignConsumptionEvidence",
    }
    roles: dict[str, list[dict[str, Any]]] = {}
    for descriptor in payload["artifacts"]:
        role = descriptor["role"]
        roles.setdefault(role, []).append(descriptor)
        content = verified[descriptor["artifact_id"]].content
        content_schema = schema["$defs"].get(definitions.get(role, ""))
        if role == "original_record":
            content_schema = schema
        if content_schema is not None and _schema_errors(content, content_schema, schema):
            errors.append(f"{role} evidence content does not match its contract")
    return roles
def _required_role_map(
    roles: dict[str, list[dict[str, Any]]], errors: list[str]
) -> dict[str, dict[str, Any]] | None:
    names = (
        "protocol", "execution", "declared_budget", "attempt_ledger",
        "access_record", "observations", "corpus_snapshot", "splits", "qrels",
        "case_list",
    )
    required: dict[str, dict[str, Any]] = {}
    for role in names:
        matches = roles.get(role, [])
        if len(matches) != 1:
            errors.append(f"evidence requires exactly one {role} artifact")
        else:
            required[role] = matches[0]
    return required if len(required) == len(names) else None
def _protocol_identity(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    required: dict[str, dict[str, Any]],
    records: dict[str, Any],
    errors: list[str],
) -> tuple[dict[str, Any], str, dict[str, Any]]:
    protocol, data, systems = records["protocol"], payload["data"], payload["systems"]
    digest = verified[required["protocol"]["artifact_id"]].digest
    sources = {key: data.get(key) for key in (
        "corpus_id", "corpus_revision", "snapshot_digest", "splits_digest",
        "qrels_digest", "case_list_digest"
    )}
    system_fields = (
        "baseline_id", "baseline_configuration_digest",
        "baseline_configuration_artifact_id", "candidate_id",
        "candidate_configuration_digest", "candidate_configuration_artifact_id",
    )
    expected_systems = {key: systems.get(key) for key in system_fields}
    if (
        payload["protocol"].get("artifact_id") != required["protocol"]["artifact_id"]
        or str(payload["protocol"].get("digest", "")).lower() != digest.lower()
        or protocol["campaign_id"] != payload["campaign_id"]
        or protocol["protocol_id"] != payload["protocol"].get("id")
        or protocol["revision"] != payload["protocol"].get("revision")
        or _timestamp(protocol["frozen_at"]) is None
        or not _same_json(protocol["systems"], expected_systems)
        or not _same_json(protocol["sources"], sources)
        or not _same_json(protocol["execution_identity"], payload["execution"])
    ):
        errors.append("frozen protocol does not bind its identity, source, systems, and execution")
    return protocol, digest, systems


def _system_configuration_context(
    systems: dict[str, Any],
    roles: dict[str, list[dict[str, Any]]],
    verified: dict[str, VerifiedArtifact],
    errors: list[str],
) -> set[str] | None:
    systems_in_scope = {systems["baseline_id"], systems["candidate_id"]}
    if (
        len(systems_in_scope) != 2
        or any(not isinstance(system_id, str) or not system_id for system_id in systems_in_scope)
    ):
        errors.append("frozen protocol needs distinct baseline and candidate systems")
        return None
    expected_configurations = {
        systems["baseline_id"]: (
            systems["baseline_configuration_artifact_id"],
            systems["baseline_configuration_digest"],
        ),
        systems["candidate_id"]: (
            systems["candidate_configuration_artifact_id"],
            systems["candidate_configuration_digest"],
        ),
    }
    configuration_systems: set[str] = set()
    descriptors = roles.get("system_configuration", [])
    if len(descriptors) != 2:
        errors.append("evidence requires one verified configuration for each comparison system")
    for descriptor in descriptors:
        artifact = verified[descriptor["artifact_id"]]
        content = artifact.content
        system_id = content.get("system_id") if isinstance(content, dict) else None
        expected = expected_configurations.get(system_id)
        if (
            expected is None
            or system_id in configuration_systems
            or descriptor["artifact_id"] != expected[0]
            or artifact.digest.lower() != str(expected[1]).lower()
        ):
            errors.append("verified system configuration bytes do not match frozen system identities")
        elif isinstance(content, dict):
            configuration_systems.add(system_id)
    if configuration_systems != systems_in_scope:
        errors.append("verified baseline and candidate configuration bytes are required")
    return systems_in_scope


def _source_case_context(
    required: dict[str, dict[str, Any]],
    records: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    errors: list[str],
) -> tuple[list[str], dict[str, str]]:
    data = records["data"]
    source_ids = {required[role]["artifact_id"] for role in _SOURCE_ROLE_TO_FIELD}
    listed = data["source_artifact_ids"]
    if len(listed) != len(source_ids) or set(listed) != source_ids:
        errors.append("source artifact list does not exactly match frozen source roles")
    for role, field in _SOURCE_ROLE_TO_FIELD.items():
        if verified[required[role]["artifact_id"]].digest.lower() != str(data.get(field, "")).lower():
            errors.append(f"{role} bytes do not match their frozen source digest")
    cases = records["case_list"]["cases"]
    case_ids = [item["case_id"] for item in cases]
    clusters = {item["case_id"]: item["cluster_id"] for item in cases}
    if (
        len(set(case_ids)) != len(case_ids)
        or data["independent_needs"] != len(case_ids)
        or data["independent_clusters"] != len(set(clusters.values()))
    ):
        errors.append("verified case list does not match independent-unit counts")
    return case_ids, clusters


def _planned_first_attempts(
    protocol: dict[str, Any],
    data: dict[str, Any],
    case_ids: list[str],
    systems_in_scope: set[str],
    errors: list[str],
) -> tuple[list[dict[str, Any]], dict[str, dict[str, Any]]] | None:
    repetitions = data["timing_repetitions"]
    if type(repetitions) is not int or repetitions <= 0:
        errors.append("frozen protocol needs positive timing repetitions")
        return None
    planned, by_slot, keys = protocol["planned_first_attempts"], {}, set()
    run_ids, attempt_ids = set(), set()
    for index, item in enumerate(planned):
        key = (item["case_id"], item["system_id"], item["repetition"])
        if (
            item["ordinal"] != index + 1
            or item["case_id"] not in case_ids
            or item["system_id"] not in systems_in_scope
            or item["repetition"] >= repetitions
            or item["slot_id"] in by_slot
            or item["run_id"] in run_ids
            or item["attempt_id"] in attempt_ids
            or key in keys
        ):
            errors.append("frozen first-attempt roster contains an invalid or duplicate slot")
            return None
        by_slot[item["slot_id"]] = item
        keys.add(key)
        run_ids.add(item["run_id"])
        attempt_ids.add(item["attempt_id"])
    expected_slots = {
        (case_id, system_id, repetition)
        for case_id in case_ids
        for system_id in systems_in_scope
        for repetition in range(repetitions)
    }
    if not expected_slots or keys != expected_slots:
        errors.append("frozen first-attempt roster is incomplete")
        return None
    return planned, by_slot


def _quality_measurements(
    by_metric: dict[str, list[dict[str, Any]]],
    systems: dict[str, Any],
    systems_in_scope: set[str],
    errors: list[str],
) -> bool:
    quality = by_metric.get("ndcg_at_10", [])
    quality_by_system = {item["system_id"]: item for item in quality}
    if (
        len(quality) != 2
        or set(quality_by_system) != systems_in_scope
        or any(
            item["scope"] != "system"
            or item["role"] != "secondary"
            or item["unit"] != "ratio"
            or item["aggregation"] != "mean"
            or item["denominator_kind"] != "independent_needs"
            or item["comparison_system_id"] is not None
            for item in quality
        )
    ):
        errors.append("confirmatory comparison requires baseline and candidate nDCG@10 evidence")
        return False
    paired = by_metric.get("ndcg_at_10_paired_difference", [])
    if (
        len(paired) != 1
        or paired[0]["scope"] != "system"
        or paired[0]["role"] != "primary"
        or paired[0]["unit"] != "ratio"
        or paired[0]["system_id"] != systems["candidate_id"]
        or paired[0]["comparison_system_id"] != systems["baseline_id"]
        or paired[0]["aggregation"] != "paired_mean_difference"
        or paired[0]["denominator_kind"] != "independent_needs"
    ):
        errors.append("confirmatory comparison requires a frozen paired primary nDCG@10 difference")
        return False
    return True


def _resource_measurements(
    by_metric: dict[str, list[dict[str, Any]]],
    systems_in_scope: set[str],
    errors: list[str],
) -> bool:
    for metric, unit, aggregation in (
        ("fast_path_p95_latency_ms", "ms", "p95"),
        ("first_document_result_p95_latency_ms", "ms", "p95"),
        ("memory_peak_bytes", "bytes", "maximum"),
        ("disk_peak_bytes", "bytes", "maximum"),
    ):
        matches = by_metric.get(metric, [])
        if (
            len(matches) != 2
            or {item["system_id"] for item in matches} != systems_in_scope
            or any(
                item["scope"] != "system"
                or item["role"] != "constraint"
                or item["unit"] != unit
                or item["aggregation"] != aggregation
                or item["denominator_kind"] != "timing_repetitions"
                or item["comparison_system_id"] is not None
                or item["max_value"] is None
                for item in matches
            )
        ):
            errors.append("confirmatory comparison requires baseline and candidate resource constraints")
            return False
    preparation = by_metric.get("preparation_wall_ms", [])
    if (
        len(preparation) != 1
        or preparation[0]["scope"] != "campaign"
        or preparation[0]["role"] != "constraint"
        or preparation[0]["unit"] != "ms"
        or preparation[0]["aggregation"] != "sum"
        or preparation[0]["denominator_kind"] != "campaign_activities"
        or preparation[0]["system_id"] is not None
        or preparation[0]["comparison_system_id"] is not None
        or preparation[0]["max_value"] is None
    ):
        errors.append("confirmatory comparison requires a preparation wall-time constraint")
        return False
    return True


def _measurement_context(
    payload: dict[str, Any],
    protocol: dict[str, Any],
    systems: dict[str, Any],
    systems_in_scope: set[str],
    errors: list[str],
) -> tuple[list[dict[str, Any]], dict[str, list[dict[str, Any]]]] | None:
    requirements = protocol["required_measurements"]
    names = [item["name"] for item in requirements]
    if (
        len(set(names)) != len(names)
        or payload["claim_eligibility"]["required_measurements"] != names
        or [item["name"] for item in payload["measurements"]] != names
    ):
        errors.append("claim-required measurements differ from the frozen protocol")
        return None
    for item in requirements:
        paired = item["aggregation"] == "paired_mean_difference"
        if paired != (item["comparison_system_id"] is not None):
            errors.append("paired aggregation and comparison-system identity must agree")
            return None
        if item["scope"] == "system":
            if item["system_id"] not in systems_in_scope:
                errors.append("system measurement references a system outside the comparison")
                return None
            if item["comparison_system_id"] is not None and (
                item["comparison_system_id"] not in systems_in_scope
                or item["comparison_system_id"] == item["system_id"]
            ):
                errors.append("paired measurement references an invalid comparison system")
                return None
        elif item["system_id"] is not None or item["comparison_system_id"] is not None or paired:
            errors.append("campaign measurement cannot claim a system identity or pair")
            return None
        if item["role"] == "constraint":
            if _decimal(item["max_value"]) is None or item["max_value"] < 0:
                errors.append("resource constraints require a finite frozen maximum")
                return None
        elif item["max_value"] is not None:
            errors.append("only constrained resource measures may declare a maximum")
            return None
    by_metric: dict[str, list[dict[str, Any]]] = {}
    for item in requirements:
        by_metric.setdefault(item["metric"], []).append(item)
    if (
        not _quality_measurements(by_metric, systems, systems_in_scope, errors)
        or not _resource_measurements(by_metric, systems_in_scope, errors)
    ):
        return None
    return requirements, by_metric


def _protocol_context(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    roles: dict[str, list[dict[str, Any]]],
    errors: list[str],
) -> dict[str, Any] | None:
    required = _required_role_map(roles, errors)
    if required is None:
        return None
    records = {role: verified[item["artifact_id"]].content for role, item in required.items()}
    records["data"] = payload["data"]
    protocol, digest, systems = _protocol_identity(
        payload, verified, required, records, errors
    )
    systems_in_scope = _system_configuration_context(systems, roles, verified, errors)
    if systems_in_scope is None:
        return None
    case_ids, clusters = _source_case_context(required, records, verified, errors)
    planned_context = _planned_first_attempts(
        protocol, payload["data"], case_ids, systems_in_scope, errors
    )
    if planned_context is None:
        return None
    planned, by_slot = planned_context
    measurement_context = _measurement_context(
        payload, protocol, systems, systems_in_scope, errors
    )
    if measurement_context is None:
        return None
    requirements, by_metric = measurement_context
    return {
        "required": required,
        "records": records,
        "protocol": protocol,
        "protocol_digest": digest,
        "freeze_id": protocol["freeze_id"],
        "frozen_at": protocol["frozen_at"],
        "campaign_id": payload["campaign_id"],
        "planned": planned,
        "plan_by_slot": by_slot,
        "requirements": requirements,
        "requirements_by_metric": by_metric,
        "clusters": clusters,
        "roles": roles,
        "systems": systems,
        "systems_in_scope": systems_in_scope,
    }
def _run_artifact_binding_errors(
    payload: dict[str, Any],
    context: dict[str, Any],
    errors: list[str],
) -> None:
    required, records = context["required"], context["records"]
    digest, campaign = context["protocol_digest"], context["campaign_id"]
    execution = records["execution"]
    if (
        execution["campaign_id"] != campaign or execution["run_id"] != payload["run_id"]
        or execution["attempt_id"] != payload["attempt_id"]
        or execution["protocol_digest"].lower() != digest.lower()
        or not _same_json(execution["execution"], payload["execution"])
    ):
        errors.append("execution record does not bind this run, attempt, and frozen protocol")
    declared, budget = records["declared_budget"], payload["budgets"]
    if (
        required["declared_budget"]["artifact_id"] != budget["declared_artifact"]
        or declared["campaign_id"] != campaign
        or declared["protocol_digest"].lower() != digest.lower()
        or declared["scope"] != budget["scope"]
        or declared["currency"] != budget["currency"]
    ):
        errors.append("declared budget does not bind campaign scope, currency, and protocol")
    access, trace = records["access_record"], payload["trace_policy"]
    if (
        access["campaign_id"] != campaign or access["run_id"] != payload["run_id"]
        or access["attempt_id"] != payload["attempt_id"]
        or access["protocol_digest"].lower() != digest.lower()
        or required["access_record"]["artifact_id"] != trace["artifact_id"]
        or not _same_json(access["trace_policy"], {
            key: value for key, value in trace.items() if key != "artifact_id"
        })
    ):
        errors.append("access record does not bind consent, access, and retention policy")
