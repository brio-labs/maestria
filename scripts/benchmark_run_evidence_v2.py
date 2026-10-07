"""Strict validator for immutable benchmark run manifests (schema v2)."""

from __future__ import annotations

import importlib.util
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

SCHEMA_PATH = (
    Path(__file__).resolve().parents[1]
    / "tests"
    / "contracts"
    / "benchmark_run_manifest_v2.schema.json"
)
_DIGEST_RE = re.compile(r"^[a-fA-F0-9]{64}$")
_COMMIT_RE = re.compile(r"^(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{64})$")


def _load_helper(module_name: str, filename: str) -> Any:
    loaded = sys.modules.get(module_name)
    if loaded is not None:
        return loaded
    module_path = Path(__file__).with_name(filename)
    spec = importlib.util.spec_from_file_location(module_name, module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load v2 helper: {filename}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        del sys.modules[module_name]
        raise
    return module


_artifact_helpers = _load_helper(
    "_sillage_benchmark_run_evidence_v2_artifacts",
    "benchmark_run_evidence_v2_artifacts.py",
)
_load_helper(
    "_sillage_benchmark_run_evidence_v2_common",
    "benchmark_run_evidence_v2_common.py",
)
_load_helper(
    "_sillage_benchmark_run_evidence_v2_protocol",
    "benchmark_run_evidence_v2_protocol.py",
)
_load_helper(
    "_sillage_benchmark_run_evidence_v2_ledger",
    "benchmark_run_evidence_v2_ledger.py",
)
_load_helper(
    "_sillage_benchmark_run_evidence_v2_measurements",
    "benchmark_run_evidence_v2_measurements.py",
)
_load_helper(
    "_sillage_benchmark_run_evidence_v2_campaign",
    "benchmark_run_evidence_v2_campaign.py",
)
_evidence_helpers = _load_helper(
    "_sillage_benchmark_run_evidence_v2_evidence",
    "benchmark_run_evidence_v2_evidence.py",
)
_view_helpers = _load_helper(
    "_sillage_benchmark_run_evidence_v2_views",
    "benchmark_run_evidence_v2_views.py",
)
_load_strict_json = _artifact_helpers._load_strict_json
_schema_errors = _artifact_helpers._schema_errors
_safe_relative_path = _artifact_helpers._safe_relative_path
_timestamp = _artifact_helpers._timestamp
_verify_artifacts = _artifact_helpers._verify_artifacts
_content_evidence_errors = _evidence_helpers._content_evidence_errors

@dataclass
class ValidationResult:
    errors: list[str]
    warnings: list[str]
    confirmatory_eligible: bool
    artifact_verification_complete: bool
    view: dict[str, Any] | None = None
    view_exit_status: int | None = None


def _mapping(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def _non_empty(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _digest(value: Any) -> bool:
    return isinstance(value, str) and _DIGEST_RE.fullmatch(value) is not None


def _validate_lifecycle(payload: dict[str, Any]) -> tuple[list[str], bool]:
    errors: list[str] = []
    lifecycle, protocol = _mapping(payload.get("lifecycle")), _mapping(payload.get("protocol"))
    state = payload.get("state")
    if not isinstance(state, str):
        return ["state: unsupported run lifecycle state"], False
    start, terminal = lifecycle.get("start_receipt"), lifecycle.get("terminal_receipt")
    started, finished = _timestamp(lifecycle.get("started_at")), _timestamp(lifecycle.get("finished_at"))
    if state in {"proposed", "not_run"}:
        if any(lifecycle.get(key) is not None for key in (
            "start_receipt", "started_at", "terminal_receipt", "finished_at"
        )):
            errors.append("lifecycle: proposed/not_run records cannot contain execution receipts")
        if state == "not_run" and not _non_empty(lifecycle.get("reason")):
            errors.append("lifecycle.reason: not_run requires a reason")
        return errors, False

    valid = True
    def reject(message: str) -> None:
        nonlocal valid
        errors.append(message)
        valid = False

    if not _non_empty(payload.get("attempt_id")):
        reject("attempt_id: execution attempts require an identity")
    if not isinstance(start, dict):
        reject("lifecycle.start_receipt: an execution requires a start receipt")
    if started is None:
        reject("lifecycle.started_at: an execution requires a timezone-aware timestamp")
    start_time = _timestamp(start.get("recorded_at")) if isinstance(start, dict) else None
    if isinstance(start, dict):
        if start_time is None:
            reject("lifecycle.start_receipt.recorded_at: invalid timestamp")
        for key, expected in (
            ("run_id", payload.get("run_id")), ("protocol_id", protocol.get("id")),
            ("protocol_revision", protocol.get("revision")),
            ("attempt_id", payload.get("attempt_id")),
        ):
            if start.get(key) != expected:
                reject(f"lifecycle.start_receipt.{key}: does not bind to this execution")
        if "status" in start or "start_receipt_id" in start:
            reject("lifecycle.start_receipt: terminal-only fields are not allowed")
        if started is not None and start_time is not None and start_time > started:
            reject("lifecycle.start_receipt: receipt must predate run start")
    if state == "unfinished":
        if terminal is not None or lifecycle.get("finished_at") is not None:
            reject("lifecycle: unfinished attempts cannot have a terminal receipt")
        if not _non_empty(lifecycle.get("reason")):
            errors.append("lifecycle.reason: unfinished attempts require an explanation")
        return errors, False
    if state not in {"complete", "failed", "timeout", "cancelled", "invalid"}:
        errors.append("state: unsupported run lifecycle state")
        return errors, False
    if not isinstance(terminal, dict):
        reject("lifecycle.terminal_receipt: terminal state requires a terminal receipt")
    if finished is None:
        reject("lifecycle.finished_at: terminal state requires a timezone-aware timestamp")
    if not _non_empty(lifecycle.get("reason")):
        reject("lifecycle.reason: terminal state requires a reason")
    terminal_time = _timestamp(terminal.get("recorded_at")) if isinstance(terminal, dict) else None
    if isinstance(terminal, dict):
        if terminal_time is None:
            reject("lifecycle.terminal_receipt.recorded_at: invalid timestamp")
        for key, expected in (
            ("run_id", payload.get("run_id")), ("protocol_id", protocol.get("id")),
            ("protocol_revision", protocol.get("revision")), ("status", state),
            ("attempt_id", payload.get("attempt_id")),
        ):
            if terminal.get(key) != expected:
                reject(f"lifecycle.terminal_receipt.{key}: does not bind to this execution")
        if isinstance(start, dict):
            if terminal.get("start_receipt_id") != start.get("receipt_id"):
                reject("lifecycle.terminal_receipt.start_receipt_id: does not link the start receipt")
            if terminal.get("attempt_id") != start.get("attempt_id"):
                reject("lifecycle.terminal_receipt.attempt_id: does not match the start receipt")
            if terminal.get("receipt_id") == start.get("receipt_id"):
                reject("lifecycle receipts must have distinct identities")
        if start_time is not None and terminal_time is not None and start_time >= terminal_time:
            reject("lifecycle receipts are not ordered: start must predate terminal")
        if finished is not None and terminal_time is not None and finished > terminal_time:
            reject("lifecycle.terminal_receipt: terminal receipt predates run finish")
    if started is not None and finished is not None and started > finished:
        reject("lifecycle: run finish predates run start")
    return errors, valid


def _validate_manifest_semantics(payload: dict[str, Any]) -> list[str]:
    errors, _ = _validate_lifecycle(payload)
    execution, data = payload["execution"], payload["data"]
    if execution["dirty_tree"] is True and not _digest(execution["patch_digest"]):
        errors.append("execution.patch_digest: dirty source requires a SHA-256 digest")
    cache = execution["cache_state"]
    if isinstance(cache, dict) and any(
        value is not None and not _non_empty(value) for value in cache.values()
    ):
        errors.append("execution.cache_state: values must be non-empty or null")

    artifact_ids: set[str] = set()
    for index, artifact in enumerate(payload["artifacts"]):
        artifact_id = artifact["artifact_id"]
        if artifact_id in artifact_ids:
            errors.append(f"artifacts[{index}].artifact_id: duplicate identity")
        artifact_ids.add(artifact_id)
        if not _safe_relative_path(artifact["path"]):
            errors.append(f"artifacts[{index}].path: must be a safe relative local path")
        access = artifact["access"]
        reason = access["reason"]
        if access["classification"] == "unavailable" and not _non_empty(reason):
            errors.append(f"artifacts[{index}].access.reason: unavailable access requires a reason")
        elif reason is not None and not _non_empty(reason):
            errors.append(f"artifacts[{index}].access.reason: must be non-empty or null")

    names: set[str] = set()
    for index, measure in enumerate(payload["measurements"]):
        name, value, status = measure["name"], measure["value"], measure["status"]
        if name in names:
            errors.append(f"measurements[{index}].name: duplicate measurement identity")
        names.add(name)
        prefix = f"measurements[{index}]"
        if status in {"unavailable", "not_run", "not_applicable", "invalid"} and value is not None:
            errors.append(f"{prefix}.value: unavailable measurements must use null")
        if status in {"measured", "estimated"} and (
            value is None or not _non_empty(measure["method"])
        ):
            errors.append(f"{prefix}: measured values require a finite value and method")
        denominator = measure.get("denominator")
        if denominator is not None:
            expected_count = data.get(denominator["kind"])
            if expected_count is not None and denominator["count"] != expected_count:
                errors.append(f"{prefix}.denominator.count: does not match the declared count")
        observation_id = measure["observations_artifact"]
        if observation_id is not None and observation_id not in artifact_ids:
            errors.append(f"{prefix}.observations_artifact: references an unknown artifact")

    correction = payload["corrections"]
    supersedes = correction["supersedes_run_id"]
    if supersedes is not None:
        if supersedes == payload["run_id"] or not _non_empty(supersedes):
            errors.append("corrections.supersedes_run_id: must identify another run")
        if not _non_empty(correction["supersedes_attempt_id"]) or not _non_empty(correction["reason"]):
            errors.append("corrections: original attempt identity and reason are required")
        matches = [item for item in payload.get("prior_evidence_refs", [])
                   if item["kind"] == "run" and item["run_id"] == supersedes
                   and item["attempt_id"] == correction["supersedes_attempt_id"]]
        if len(matches) != 1:
            errors.append("corrections: original run and attempt need one evidence reference")
    elif correction["supersedes_attempt_id"] is not None or correction["reason"] is not None:
        errors.append("corrections: original identity and reason require a superseded run")
    return errors


def _eligibility_reasons(payload: dict[str, Any]) -> list[str]:
    reasons: list[str] = []
    protocol = payload["protocol"]
    execution = payload["execution"]
    data = payload["data"]
    systems = payload["systems"]
    budgets = payload["budgets"]
    trace = payload["trace_policy"]
    provenance = payload["agent_provenance"]
    if payload.get("example_only") is True:
        reasons.append("example record")
    if payload.get("state") != "complete":
        reasons.append("run is not complete")
    if payload.get("experiment_type") != "confirmatory":
        reasons.append("run is not confirmatory")
    if protocol.get("frozen") is not True or not _digest(protocol.get("digest")):
        reasons.append("protocol is not frozen and digest-bound")
    if not _non_empty(payload.get("campaign_id")) or not _non_empty(payload.get("attempt_id")):
        reasons.append("campaign or attempt identity is missing")
    if not _validate_lifecycle(payload)[1]:
        reasons.append("start/terminal receipts are incomplete or unbound")

    cache = execution.get("cache_state")
    cache_boundaries = ("process", "index", "model", "application", "os", "remote")
    if (
        not isinstance(cache, dict)
        or any(
            not _non_empty(cache.get(boundary))
            or cache[boundary].strip().lower() in {"unknown", "unspecified"}
            for boundary in cache_boundaries
        )
    ):
        reasons.append("all six product and OS cache boundaries must be explicit")
    if execution.get("dirty_tree") is not False:
        reasons.append("confirmatory execution requires a clean source tree")
    code_commit = execution.get("code_commit")
    if not isinstance(code_commit, str) or not _COMMIT_RE.fullmatch(code_commit):
        reasons.append("source commit identity is incomplete")
    if not all(_digest(execution.get(key)) for key in (
        "lockfile_digest", "configuration_digest", "environment_digest"
    )):
        reasons.append("build or environment identity is incomplete")
    if not all(_non_empty(execution.get(key)) for key in (
        "toolchain", "build_profile", "command", "source_policy",
        "persistence_policy", "instrumentation"
    )):
        reasons.append("execution provenance is incomplete")
    if not all(_non_empty(execution.get("hardware", {}).get(key)) for key in (
        "cpu", "os", "power_mode"
    )) or (
        type(execution.get("hardware", {}).get("ram_bytes")) is not int
        or execution["hardware"]["ram_bytes"] <= 0
    ):
        reasons.append("hardware identity is incomplete")
    if not all(execution.get("runtime", {}).get(key) is not None for key in (
        "providers", "model_revisions", "tokenizer_revisions",
        "export_revisions", "quantization"
    )) or execution.get("seeds") is None:
        reasons.append("runtime or seed identity is incomplete")
    if not all(_non_empty(data.get(key)) for key in ("corpus_id", "corpus_revision")):
        reasons.append("source identity is incomplete")
    if not all(_non_empty(systems.get(key)) for key in ("baseline_id", "candidate_id")):
        reasons.append("comparison systems are incomplete")
    if not all(_non_empty(budgets.get(key)) for key in ("scope", "currency")):
        reasons.append("budget scope or currency is incomplete")
    if not all(_non_empty(trace.get(key)) for key in (
        "origin", "retention_policy", "access_policy"
    )) or any(trace.get(key) is None for key in (
        "product_telemetry_opt_in", "private_query_content_logged",
        "public_release_authorized"
    )):
        reasons.append("trace, access, or retention policy is incomplete")

    origin = trace.get("origin")
    telemetry_opt_in = trace.get("product_telemetry_opt_in")
    if (
        origin == "product_telemetry" and telemetry_opt_in is not True
    ) or (
        origin == "explicitly_started_experiment"
        and type(telemetry_opt_in) is not bool
    ) or origin not in {"product_telemetry", "explicitly_started_experiment"}:
        reasons.append("trace origin does not have the required affirmative consent")
    if not all(_non_empty(provenance.get(key)) for key in (
        "task_id", "tool_versions", "input_revision", "reviewer"
    )) or provenance.get("chain_of_thought_included") is not False:
        reasons.append("reviewable execution provenance is incomplete")
    claim = payload["claim_eligibility"]
    if claim.get("status") != "eligible" or not claim.get("claim_ids") or not _non_empty(claim.get("reason")):
        reasons.append("manifest does not declare a supported claim")
    measures = {item["name"]: item for item in payload["measurements"]}
    if any(
        name not in measures
        or measures[name]["status"] != "measured"
        or measures[name]["value"] is None
        for name in claim["required_measurements"]
    ):
        reasons.append("required measurements are unavailable")
    return reasons


def validate_manifest(
    manifest: Path,
    artifact_root: Path | None = None,
    view_kind: str | None = None,
) -> ValidationResult:
    def invalid_result(errors: list[str], diagnostic: str) -> ValidationResult:
        view = _view_helpers.blocked_view(view_kind, diagnostic) if view_kind else None
        return ValidationResult(errors, [], False, False, view, 1 if view is not None else None)

    if view_kind not in {None, "references", "claims"}:
        return invalid_result(["unsupported derived view"], "unsupported_view")
    try:
        payload = _load_strict_json(manifest)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError, RecursionError):
        return invalid_result(["v2 manifest cannot be read as strict JSON"], "manifest_invalid")
    if not isinstance(payload, dict):
        return invalid_result(["v2 manifest root must be an object"], "manifest_invalid")
    try:
        schema = _load_strict_json(SCHEMA_PATH)
    except (OSError, UnicodeError, json.JSONDecodeError, ValueError, RecursionError):
        return invalid_result(["v2 JSON Schema is unavailable or invalid"], "schema_unavailable")
    if not isinstance(schema, dict):
        return invalid_result(["v2 JSON Schema root must be an object"], "schema_unavailable")

    try:
        errors = _schema_errors(payload, schema, schema)
    except (ArithmeticError, KeyError, RecursionError, TypeError, ValueError):
        return invalid_result(["v2 manifest does not satisfy its JSON Schema"], "manifest_invalid")
    if errors:
        return invalid_result(errors, "manifest_invalid")
    errors = _validate_manifest_semantics(payload)
    if errors:
        return invalid_result(errors, "manifest_invalid")
    artifacts = payload["artifacts"]
    verified, artifact_errors, warnings = _verify_artifacts(artifacts, artifact_root)
    complete = artifact_root is not None and bool(artifacts) and len(verified) == len(artifacts)
    reference_index = _view_helpers.build_reference_index(
        payload, verified, artifact_errors, warnings, artifact_root is not None
    )
    required_references_unverified = any(
        item["provides_measurements_for_this_run"] and item["status"] != "verified"
        for item in reference_index["references"]
    )
    declared = payload["claim_eligibility"]["status"] == "eligible"
    view_source: dict[str, Any] | None = None
    if artifact_root is None:
        warnings.append("artifact verification was not requested; the run is not artifact-verified")
        if declared:
            errors.append("claim_eligibility: artifact evidence was not verified")
    else:
        errors.extend(artifact_errors)
        if view_kind is not None:
            view_source = {}
        if not artifact_errors and complete and (declared or view_kind is not None):
            content_errors = _content_evidence_errors(
                payload,
                verified,
                schema,
                _validate_manifest_semantics,
                view_source=view_source,
                allow_noncomplete=view_kind is not None,
            )
            if declared:
                errors.extend(content_errors)
        if declared and (artifact_errors or not complete):
            errors.append("claim_eligibility: required artifact evidence is unavailable")
    if declared and required_references_unverified:
        errors.append(
            "claim_eligibility: measurement-bearing evidence reference is unavailable or unverified"
        )
    reasons = _eligibility_reasons(payload) if declared else []
    if declared and reasons:
        errors.append("claim_eligibility: claimed eligibility is not supported by the required evidence")
    eligible = declared and not errors and not reasons
    if not eligible and not declared:
        warnings.append("confirmatory eligibility is not established")

    view: dict[str, Any] | None = None
    view_exit_status: int | None = None
    if view_kind == "references":
        content_invalid = (
            complete
            and view_source is not None
            and _view_helpers.content_is_invalid(view_source)
        )
        view = (
            _view_helpers.blocked_view("references", "evidence_content_invalid")
            if content_invalid else reference_index
        )
        invalid_references = any(
            item["status"] == "invalid" for item in reference_index["references"]
        )
        view_exit_status = (
            1
            if (
                not complete or artifact_errors or content_invalid
                or required_references_unverified or invalid_references
            )
            else 0
        )
    elif view_kind == "claims":
        view_reasons = _eligibility_reasons(payload)
        if required_references_unverified:
            view_reasons.append(
                "measurement-bearing evidence reference is unavailable or unverified"
            )
        blocked = (
            ["artifact_verification_incomplete"]
            if artifact_root is None or artifact_errors or not complete
            else []
        )
        if required_references_unverified:
            blocked.append("measurement_reference_unverified")
        view, view_exit_status = _view_helpers.build_claim_view(
            payload, view_source, eligible, view_reasons, blocked, reference_index
        )
    return ValidationResult(
        errors, list(dict.fromkeys(warnings)), eligible, complete, view, view_exit_status
    )


def errors_for_manifest(manifest: Path, artifact_root: Path | None = None) -> list[str]:
    """Return safe structural/artifact errors without exposing evidence values."""
    return validate_manifest(manifest, artifact_root).errors
