from __future__ import annotations

import hashlib
import importlib.util
import io
import json
from decimal import Decimal
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "benchmark_evidence.py"
SPEC = importlib.util.spec_from_file_location("benchmark_evidence", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("unable to load benchmark_evidence.py")
EVIDENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EVIDENCE)
MANIFEST = ROOT / "tests" / "contracts" / "benchmark_evidence_v1.json"


def v2_proposed_manifest() -> dict[str, object]:
    return {
        "schema_version": 2,
        "record_kind": "benchmark_run_manifest",
        "example_only": True,
        "campaign_id": None,
        "run_id": "proposal-1",
        "state": "proposed",
        "experiment_type": "confirmatory",
        "protocol": {"id": "protocol-1", "revision": "draft", "digest": None, "frozen": False},
        "lifecycle": {
            "start_receipt": None,
            "started_at": None,
            "terminal_receipt": None,
            "finished_at": None,
            "reason": "No run was started.",
        },
        "execution": {
            "code_commit": None,
            "dirty_tree": None,
            "patch_digest": None,
            "lockfile_digest": None,
            "toolchain": None,
            "build_profile": None,
            "command": None,
            "configuration_digest": None,
            "environment_digest": None,
            "hardware": {"cpu": None, "ram_bytes": None, "gpu": None, "os": None, "power_mode": None},
            "runtime": {
                "providers": None,
                "model_revisions": None,
                "tokenizer_revisions": None,
                "export_revisions": None,
                "quantization": None,
            },
            "seeds": None,
            "order_artifact": None,
            "cache_state": None,
            "source_policy": None,
            "persistence_policy": None,
            "instrumentation": None,
        },
        "data": {
            "corpus_id": None,
            "corpus_revision": None,
            "snapshot_digest": None,
            "splits_digest": None,
            "qrels_digest": None,
            "case_list_digest": None,
            "independent_needs": None,
            "independent_clusters": None,
            "source_artifact_ids": [],
            "timing_repetitions": None,
        },
        "systems": {
            "baseline_id": None,
            "baseline_configuration_digest": None,
            "baseline_configuration_artifact_id": None,
            "candidate_id": None,
            "candidate_configuration_digest": None,
            "candidate_configuration_artifact_id": None,
        },
        "budgets": {
            "declared_artifact": None,
            "actual_compute_seconds": None,
            "actual_cost_amount": None,
            "currency": None,
            "scope": None,
        },
        "measurements": [
            {
                "name": "quality",
                "role": "primary",
                "value": None,
                "unit": "ratio",
                "status": "not_run",
                "reason": "No observations.",
                "method": None,
                "interval": None,
                "observations_artifact": None,
            }
        ],
        "artifacts": [],
        "prior_evidence_refs": [],
        "trace_policy": {
            "origin": "explicitly_started_experiment",
            "product_telemetry_opt_in": None,
            "private_query_content_logged": None,
            "retention_policy": None,
            "access_policy": None,
            "public_release_authorized": False,
            "artifact_id": None,
        },
        "agent_provenance": {
            "task_id": None,
            "agent_model_id": None,
            "tool_versions": None,
            "input_revision": None,
            "reviewer": None,
            "chain_of_thought_included": False,
        },
        "claim_eligibility": {
            "status": "proposed",
            "reason": "No execution or artifact verification.",
            "claim_ids": [],
            "required_measurements": [],
            "product_promotion": "not_assessed",
        },
        "corrections": {
            "supersedes_run_id": None, "supersedes_attempt_id": None, "reason": None
        },
    }


def add_artifact(
    payload: dict[str, object],
    path: str,
    expected_bytes: bytes,
    role: str = "supplemental",
    artifact_id: str | None = None,
    media_type: str = "application/json",
) -> None:
    artifacts = payload["artifacts"]
    assert isinstance(artifacts, list)
    artifact_id = artifact_id or f"artifact-{len(artifacts) + 1}"
    artifacts.append(
        {
            "artifact_id": artifact_id,
            "role": role,
            "path": path,
            "sha256": hashlib.sha256(expected_bytes).hexdigest(),
            "size_bytes": len(expected_bytes),
            "media_type": media_type,
            "access": {"classification": "public", "reason": None},
            "retention_policy": "controlled test fixture",
        }
    )


def _canonical_json(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _write_v2_json_artifact(
    payload: dict[str, object],
    root: Path,
    artifact_id: str,
    filename: str,
    role: str,
    record: dict[str, object],
) -> tuple[bytes, str]:
    content = _canonical_json(record)
    add_artifact(payload, filename, content, role, artifact_id)
    (root / filename).write_bytes(content)
    return content, hashlib.sha256(content).hexdigest()


def _add_v2_receipts(
    payload: dict[str, object],
    root: Path,
    protocol_digest: str,
    freeze_id: str,
    frozen_at: str,
    label: str,
    run_id: str,
    attempt_id: str,
    status: str,
    start_time: str,
    terminal_time: str,
) -> dict[str, object]:
    start_id, terminal_id = f"start-{label}", f"terminal-{label}"
    start = {
        "evidence_version": 1, "kind": "start_receipt", "receipt_id": start_id,
        "attempt_id": attempt_id, "run_id": run_id, "protocol_id": "protocol-1",
        "protocol_revision": "rev-a", "protocol_digest": protocol_digest,
        "freeze_id": freeze_id, "frozen_at": frozen_at, "recorded_at": start_time,
    }
    terminal = {
        "evidence_version": 1, "kind": "terminal_receipt", "receipt_id": terminal_id,
        "attempt_id": attempt_id, "run_id": run_id, "protocol_id": "protocol-1",
        "protocol_revision": "rev-a", "recorded_at": terminal_time,
        "status": status, "start_receipt_id": start_id,
    }
    _, start_digest = _write_v2_json_artifact(
        payload, root, start_id, f"{start_id}.json", "start_receipt", start
    )
    _, terminal_digest = _write_v2_json_artifact(
        payload, root, terminal_id, f"{terminal_id}.json", "terminal_receipt", terminal
    )
    return {
        "start_id": start_id,
        "terminal_id": terminal_id,
        "start": {
            key: value for key, value in start.items()
            if key not in {"evidence_version", "kind"}
        } | {"digest": start_digest},
        "terminal": {
            key: value for key, value in terminal.items()
            if key not in {"evidence_version", "kind"}
        } | {"digest": terminal_digest},
    }


def _v2_measurement_requirements() -> list[dict[str, object]]:
    requirements: list[dict[str, object]] = []
    for system_id in ("baseline", "candidate"):
        requirements.append({
            "name": f"ndcg_at_10_{system_id}",
            "metric": "ndcg_at_10",
            "role": "secondary",
            "unit": "ratio",
            "scope": "system",
            "system_id": system_id,
            "comparison_system_id": None,
            "denominator_kind": "independent_needs",
            "aggregation": "mean",
            "max_value": None,
        })
    requirements.append({
        "name": "ndcg_at_10_paired_difference",
        "metric": "ndcg_at_10_paired_difference",
        "role": "primary",
        "unit": "ratio",
        "scope": "system",
        "system_id": "candidate",
        "comparison_system_id": "baseline",
        "denominator_kind": "independent_needs",
        "aggregation": "paired_mean_difference",
        "max_value": None,
    })
    for metric, unit, aggregation, maximum in (
        ("fast_path_p95_latency_ms", "ms", "p95", 100.0),
        ("first_document_result_p95_latency_ms", "ms", "p95", 120.0),
        ("memory_peak_bytes", "bytes", "maximum", 4096.0),
        ("disk_peak_bytes", "bytes", "maximum", 8192.0),
    ):
        for system_id in ("baseline", "candidate"):
            requirements.append({
                "name": f"{metric}_{system_id}",
                "metric": metric,
                "role": "constraint",
                "unit": unit,
                "scope": "system",
                "system_id": system_id,
                "comparison_system_id": None,
                "denominator_kind": "timing_repetitions",
                "aggregation": aggregation,
                "max_value": maximum,
            })
    requirements.append({
        "name": "preparation_wall_ms",
        "metric": "preparation_wall_ms",
        "role": "constraint",
        "unit": "ms",
        "scope": "campaign",
        "system_id": None,
        "comparison_system_id": None,
        "denominator_kind": "campaign_activities",
        "aggregation": "sum",
        "max_value": 100.0,
    })
    return requirements


def _add_v2_campaign_consumptions(
    payload: dict[str, object],
    root: Path,
    protocol_digest: str,
    freeze_id: str,
    frozen_at: str,
) -> list[dict[str, object]]:
    activity_specs = (
        ("preparation", "preparation", 1, "run-preparation", "attempt-preparation",
         "complete", "build retrieval index", 0.2, 0.02, 40.0, "08:59:10", "08:59:11"),
        ("tuning", "tuning", 1, "run-tuning", "attempt-tuning",
         "complete", "tune retrieval parameters", 0.1, 0.01, 12.0, "08:59:20", "08:59:21"),
        ("setup", "setup", 1, "run-setup-1", "attempt-setup-1",
         "failed", "initialize candidate runtime", 0.15, 0.015, 8.0, "08:59:30", "08:59:31"),
        ("setup", "setup", 2, "run-setup-2", "attempt-setup-2",
         "complete", "retry candidate initialization", 0.05, 0.005, 4.0, "08:59:40", "08:59:41"),
    )
    rows: list[dict[str, object]] = []
    for label, phase, number, run_id, attempt_id, state, activity, compute, cost, wall, start, end in activity_specs:
        receipts = _add_v2_receipts(
            payload, root, protocol_digest, freeze_id, frozen_at,
            f"consumption-{label}-{number}", run_id, attempt_id, state,
            f"2026-10-07T{start}Z", f"2026-10-07T{end}Z",
        )
        artifact_id = f"consumption-{label}-{number}"
        fields: dict[str, object] = {
            "consumption_id": label,
            "phase": phase,
            "attempt_number": number,
            "run_id": run_id,
            "attempt_id": attempt_id,
            "state": state,
            "activity": activity,
            "compute_seconds": compute,
            "cost_amount": cost,
            "wall_time_ms": wall,
            "start_receipt_artifact_id": receipts["start_id"],
            "terminal_receipt_artifact_id": receipts["terminal_id"],
        }
        record = {
            "evidence_version": 1,
            "kind": "campaign_consumption",
            "campaign_id": payload["campaign_id"],
            "protocol_digest": protocol_digest,
            **fields,
        }
        _write_v2_json_artifact(
            payload, root, artifact_id, f"{artifact_id}.json",
            "campaign_consumption", record,
        )
        rows.append({"artifact_id": artifact_id, **fields})
    return rows


def _add_v2_evaluation_attempts(
    payload: dict[str, object],
    root: Path,
    protocol_digest: str,
    freeze_id: str,
    frozen_at: str,
    include_failed_retry: bool,
) -> tuple[list[dict[str, object]], dict[str, dict[str, object]], dict[str, object]]:
    baseline_state = "failed" if include_failed_retry else "complete"
    baseline_quality = 0.0 if include_failed_retry else 0.4
    specs = [
        ("baseline", "slot-baseline", "run-baseline", "attempt-baseline", 1,
         baseline_state, baseline_quality, 0.4, 0.04, "09:00:00", "09:00:02"),
        ("candidate", "slot-candidate", "run-current", "attempt-current", 1,
         "complete", 0.6, 0.6, 0.06, "10:00:00", "10:00:03"),
    ]
    if include_failed_retry:
        specs.append((
            "baseline", "slot-baseline", "run-baseline-retry",
            "attempt-baseline-retry", 2, "complete", 0.4, 0.25, 0.025,
            "10:05:00", "10:05:03",
        ))
    attempts: list[dict[str, object]] = []
    outcomes: dict[str, dict[str, object]] = {}
    candidate_receipts: dict[str, object] | None = None
    for ordinal, spec in enumerate(specs, 1):
        system_id, slot_id, run_id, attempt_id, number, state, quality, compute, cost, start, end = spec
        receipts = _add_v2_receipts(
            payload, root, protocol_digest, freeze_id, frozen_at,
            f"evaluation-{number}-{system_id}", run_id, attempt_id, state,
            f"2026-10-07T{start}Z", f"2026-10-07T{end}Z",
        )
        if system_id == "candidate":
            candidate_receipts = receipts
        if number == 1:
            outcomes[system_id] = {
                "slot_id": slot_id,
                "run_id": run_id,
                "attempt_id": attempt_id,
                "state": state,
                "ndcg_at_10": quality,
                "values": {
                    "fast_path_p95_latency_ms": 40.0 if system_id == "baseline" else 30.0,
                    "first_document_result_p95_latency_ms": 75.0 if system_id == "baseline" else 65.0,
                    "memory_peak_bytes": 1024.0 if system_id == "baseline" else 1280.0,
                    "disk_peak_bytes": 4096.0 if system_id == "baseline" else 3584.0,
                },
            }
        attempts.append({
            "ordinal": ordinal,
            "slot_id": slot_id,
            "attempt_number": number,
            "case_id": "case-1",
            "system_id": system_id,
            "repetition": 0,
            "seed": 11 if system_id == "baseline" else 12,
            "run_id": run_id,
            "attempt_id": attempt_id,
            "state": state,
            "compute_seconds": compute,
            "cost_amount": cost,
            "start_receipt_artifact_id": receipts["start_id"],
            "terminal_receipt_artifact_id": receipts["terminal_id"],
        })
    if candidate_receipts is None:
        raise AssertionError("synthetic fixture must include its candidate first attempt")
    return attempts, outcomes, candidate_receipts


def _build_v2_observations(
    requirements: list[dict[str, object]],
    outcomes: dict[str, dict[str, object]],
    consumptions: list[dict[str, object]],
    campaign_id: str,
    protocol_digest: str,
    ledger_digest: str,
) -> tuple[dict[str, object], list[dict[str, object]]]:
    observation_measures: list[dict[str, object]] = []
    manifest_measures: list[dict[str, object]] = []
    for requirement in requirements:
        name = requirement["name"]
        metric = requirement["metric"]
        system_id = requirement["system_id"]
        aggregation = requirement["aggregation"]
        scope = requirement["scope"]
        if scope == "campaign":
            activity_rows = [row for row in consumptions if row["phase"] == "preparation"]
            value = sum(float(row["wall_time_ms"]) for row in activity_rows)
            denominator_count = len(activity_rows)
            observation_artifact = None
        else:
            outcome = outcomes[str(system_id)]
            if aggregation == "paired_mean_difference":
                value = float(
                    Decimal(str(outcomes["candidate"]["ndcg_at_10"]))
                    - Decimal(str(outcomes["baseline"]["ndcg_at_10"]))
                )
            elif metric == "ndcg_at_10":
                value = float(outcome["ndcg_at_10"])
            else:
                value = float(outcome["values"][str(metric)])
            observation_measures.append({
                "name": name,
                "interval": None,
                "observations": [{
                    "slot_id": outcome["slot_id"],
                    "run_id": outcome["run_id"],
                    "attempt_id": outcome["attempt_id"],
                    "state": outcome["state"],
                    "value": value,
                }],
            })
            denominator_count = 1
            observation_artifact = "observations"
        manifest_measures.append({
            "name": name,
            "role": requirement["role"],
            "value": value,
            "unit": requirement["unit"],
            "status": "measured",
            "reason": "synthetic first-attempt evidence",
            "method": aggregation,
            "denominator": {
                "kind": requirement["denominator_kind"],
                "count": denominator_count,
            },
            "interval": None,
            "observations_artifact": observation_artifact,
        })
    evidence = {
        "evidence_version": 1,
        "kind": "observations",
        "campaign_id": campaign_id,
        "protocol_digest": protocol_digest,
        "attempt_ledger_digest": ledger_digest,
        "measurements": observation_measures,
    }
    return evidence, manifest_measures


def _add_v2_source_and_configuration_records(
    payload: dict[str, object], root: Path
) -> dict[str, object]:
    data = payload["data"]
    source_bytes = {
        "corpus_snapshot": ("corpus.bin", b"synthetic corpus bytes\n", "application/octet-stream"),
        "splits": ("splits.csv", b"case_id,split\ncase-1,test\n", "text/csv"),
        "qrels": ("qrels.tsv", b"case-1\titem-1\t1\n", "text/tab-separated-values"),
    }
    case_list = {
        "evidence_version": 1,
        "kind": "case_list",
        "cases": [{"case_id": "case-1", "cluster_id": "cluster-1"}],
    }
    source_bytes["case_list"] = ("cases.json", _canonical_json(case_list), "application/json")
    source_ids: list[str] = []
    source_digests: dict[str, str] = {}
    for role, (filename, content, media_type) in source_bytes.items():
        artifact_id = f"source-{role}"
        add_artifact(payload, filename, content, role, artifact_id, media_type)
        (root / filename).write_bytes(content)
        source_ids.append(artifact_id)
        source_digests[role] = hashlib.sha256(content).hexdigest()
    data.update({
        "corpus_id": "synthetic-corpus",
        "corpus_revision": "rev-a",
        "snapshot_digest": source_digests["corpus_snapshot"],
        "splits_digest": source_digests["splits"],
        "qrels_digest": source_digests["qrels"],
        "case_list_digest": source_digests["case_list"],
        "independent_needs": 1,
        "independent_clusters": 1,
        "timing_repetitions": 1,
        "source_artifact_ids": source_ids,
    })
    systems: dict[str, object] = {"baseline_id": "baseline", "candidate_id": "candidate"}
    for system_id, implementation in (
        ("baseline", "synthetic baseline"),
        ("candidate", "synthetic candidate"),
    ):
        artifact_id = f"configuration-{system_id}"
        _, digest = _write_v2_json_artifact(
            payload, root, artifact_id, f"{artifact_id}.json", "system_configuration",
            {
                "evidence_version": 1,
                "kind": "system_configuration",
                "system_id": system_id,
                "implementation": implementation,
                "configuration": {"mode": "synthetic", "system": system_id},
            },
        )
        systems[f"{system_id}_configuration_artifact_id"] = artifact_id
        systems[f"{system_id}_configuration_digest"] = digest
    payload["systems"] = systems
    return systems


def _v2_execution_identity(
    cache_state: dict[str, object] | None = None,
    ram_bytes: int = 1024,
) -> dict[str, object]:
    return {
        "code_commit": "1" * 40,
        "dirty_tree": False,
        "patch_digest": None,
        "lockfile_digest": "2" * 64,
        "toolchain": "synthetic-test-toolchain",
        "build_profile": "release",
        "command": "synthetic test command",
        "configuration_digest": "3" * 64,
        "environment_digest": "4" * 64,
        "hardware": {
            "cpu": "synthetic-cpu", "ram_bytes": ram_bytes, "gpu": None,
            "os": "synthetic-os", "power_mode": "controlled",
        },
        "runtime": {
            "providers": ["local"], "model_revisions": ["model-a"],
            "tokenizer_revisions": ["tokenizer-a"], "export_revisions": [],
            "quantization": "none",
        },
        "seeds": [11, 12],
        "order_artifact": "attempt-ledger",
        "cache_state": cache_state if cache_state is not None else {
            "process": "fresh", "index": "fresh", "model": "warm",
            "application": "fresh", "os": "cold", "remote": "unused",
        },
        "source_policy": "synthetic-only",
        "persistence_policy": "test-only",
        "instrumentation": "synthetic fixture",
    }


def _add_v2_protocol_and_budget(
    payload: dict[str, object],
    root: Path,
    systems: dict[str, object],
    requirements: list[dict[str, object]],
    planned: list[dict[str, object]],
    max_concurrent_attempts: int,
) -> tuple[str, str, str, str]:
    freeze_id, frozen_at = "freeze-1", "2026-10-07T08:59:00Z"
    data = payload["data"]
    protocol = {
        "evidence_version": 1,
        "kind": "protocol",
        "campaign_id": payload["campaign_id"],
        "protocol_id": "protocol-1",
        "revision": "rev-a",
        "freeze_id": freeze_id,
        "frozen_at": frozen_at,
        "max_concurrent_attempts": max_concurrent_attempts,
        "sources": {
            "corpus_id": data["corpus_id"],
            "corpus_revision": data["corpus_revision"],
            "snapshot_digest": data["snapshot_digest"],
            "splits_digest": data["splits_digest"],
            "qrels_digest": data["qrels_digest"],
            "case_list_digest": data["case_list_digest"],
        },
        "systems": systems,
        "execution_identity": payload["execution"],
        "required_measurements": requirements,
        "planned_first_attempts": planned,
    }
    _, protocol_digest = _write_v2_json_artifact(
        payload, root, "protocol", "protocol.json", "protocol", protocol
    )
    payload["protocol"] = {
        "id": "protocol-1",
        "revision": "rev-a",
        "digest": protocol_digest,
        "frozen": True,
        "artifact_id": "protocol",
    }
    budget_record = {
        "evidence_version": 1,
        "kind": "declared_budget",
        "campaign_id": payload["campaign_id"],
        "protocol_digest": protocol_digest,
        "scope": "campaign",
        "currency": "USD",
        "max_compute_seconds": 5.0,
        "max_cost_amount": 2.0,
    }
    _, budget_digest = _write_v2_json_artifact(
        payload, root, "declared-budget", "declared-budget.json",
        "declared_budget", budget_record,
    )
    return protocol_digest, budget_digest, freeze_id, frozen_at


def _add_v2_access_record(
    payload: dict[str, object], root: Path, protocol_digest: str
) -> None:
    payload["trace_policy"] = {
        "origin": "explicitly_started_experiment",
        "product_telemetry_opt_in": False,
        "private_query_content_logged": False,
        "retention_policy": "test-only",
        "access_policy": "synthetic-only",
        "public_release_authorized": False,
        "artifact_id": "access-record",
    }
    access_record = {
        "evidence_version": 1,
        "kind": "access_record",
        "campaign_id": payload["campaign_id"],
        "run_id": payload["run_id"],
        "attempt_id": payload["attempt_id"],
        "protocol_digest": protocol_digest,
        "trace_policy": {
            key: value for key, value in payload["trace_policy"].items()
            if key != "artifact_id"
        },
    }
    _write_v2_json_artifact(
        payload, root, "access-record", "access-record.json",
        "access_record", access_record,
    )


def _add_v2_attempt_ledger(
    payload: dict[str, object],
    root: Path,
    protocol_digest: str,
    budget_digest: str,
    freeze_id: str,
    frozen_at: str,
    planned: list[dict[str, object]],
    include_failed_retry: bool,
) -> tuple[str, dict[str, dict[str, object]], dict[str, object], list[dict[str, object]]]:
    consumptions = _add_v2_campaign_consumptions(
        payload, root, protocol_digest, freeze_id, frozen_at
    )
    attempts, outcomes, candidate_receipts = _add_v2_evaluation_attempts(
        payload, root, protocol_digest, freeze_id, frozen_at, include_failed_retry
    )
    all_rows = (*attempts, *consumptions)
    compute_total = sum(
        (Decimal(str(row["compute_seconds"])) for row in all_rows), Decimal(0)
    )
    cost_total = sum(
        (Decimal(str(row["cost_amount"])) for row in all_rows), Decimal(0)
    )
    ledger = {
        "evidence_version": 1,
        "kind": "attempt_ledger",
        "campaign_id": payload["campaign_id"],
        "protocol_digest": protocol_digest,
        "case_list_digest": payload["data"]["case_list_digest"],
        "declared_budget_digest": budget_digest,
        "currency": "USD",
        "first_attempt_count": len(planned),
        "total_compute_seconds": float(compute_total),
        "total_cost_amount": float(cost_total),
        "attempts": attempts,
        "consumptions": consumptions,
    }
    _, ledger_digest = _write_v2_json_artifact(
        payload, root, "attempt-ledger", "attempt-ledger.json",
        "attempt_ledger", ledger,
    )
    _write_v2_json_artifact(
        payload, root, "execution", "execution.json", "execution",
        {
            "evidence_version": 1,
            "kind": "execution",
            "campaign_id": payload["campaign_id"],
            "run_id": payload["run_id"],
            "attempt_id": payload["attempt_id"],
            "protocol_digest": protocol_digest,
            "execution": payload["execution"],
        },
    )
    payload["budgets"] = {
        "declared_artifact": "declared-budget",
        "actual_compute_seconds": float(compute_total),
        "actual_cost_amount": float(cost_total),
        "currency": "USD",
        "scope": "campaign",
    }
    return ledger_digest, outcomes, candidate_receipts, consumptions


def complete_v2_run(
    root: Path,
    include_failed_retry: bool = False,
    max_concurrent_attempts: int = 1,
    cache_state: dict[str, object] | None = None,
    ram_bytes: int = 1024,
) -> dict[str, object]:
    root.mkdir(parents=True, exist_ok=True)
    payload = v2_proposed_manifest()
    payload.update({
        "example_only": False,
        "campaign_id": "synthetic-campaign",
        "run_id": "run-current",
        "attempt_id": "attempt-current",
        "state": "complete",
        "experiment_type": "confirmatory",
    })
    systems = _add_v2_source_and_configuration_records(payload, root)
    payload["execution"] = _v2_execution_identity(cache_state, ram_bytes)
    planned = [
        {"ordinal": 1, "slot_id": "slot-baseline", "case_id": "case-1",
         "system_id": "baseline", "repetition": 0, "seed": 11,
         "run_id": "run-baseline", "attempt_id": "attempt-baseline"},
        {"ordinal": 2, "slot_id": "slot-candidate", "case_id": "case-1",
         "system_id": "candidate", "repetition": 0, "seed": 12,
         "run_id": "run-current", "attempt_id": "attempt-current"},
    ]
    requirements = _v2_measurement_requirements()
    protocol_digest, budget_digest, freeze_id, frozen_at = _add_v2_protocol_and_budget(
        payload, root, systems, requirements, planned, max_concurrent_attempts
    )
    _add_v2_access_record(payload, root, protocol_digest)
    ledger_digest, outcomes, candidate_receipts, consumptions = _add_v2_attempt_ledger(
        payload, root, protocol_digest, budget_digest, freeze_id,
        frozen_at, planned, include_failed_retry,
    )
    observation_record, payload["measurements"] = _build_v2_observations(
        requirements, outcomes, consumptions, str(payload["campaign_id"]),
        protocol_digest, ledger_digest,
    )
    _write_v2_json_artifact(
        payload, root, "observations", "observations.json",
        "observations", observation_record,
    )
    payload["lifecycle"] = {
        "start_receipt": candidate_receipts["start"],
        "started_at": "2026-10-07T10:00:01Z",
        "terminal_receipt": candidate_receipts["terminal"],
        "finished_at": "2026-10-07T10:00:02Z",
        "reason": "synthetic test fixture complete",
    }
    payload["claim_eligibility"] = {
        "status": "eligible",
        "reason": "synthetic validator test",
        "claim_ids": ["synthetic-claim"],
        "required_measurements": [item["name"] for item in requirements],
        "product_promotion": "not_assessed",
    }
    payload["agent_provenance"] = {
        "task_id": "synthetic-test",
        "agent_model_id": "synthetic",
        "tool_versions": "fixture",
        "input_revision": "fixture",
        "reviewer": "synthetic reviewer",
        "chain_of_thought_included": False,
    }
    return payload


def add_original_reference(
    payload: dict[str, object], root: Path, include_original: bool
) -> None:
    original = v2_proposed_manifest()
    original.update({"run_id": "run-original", "attempt_id": "attempt-original"})
    original_bytes = _canonical_json(original)
    digest = hashlib.sha256(original_bytes).hexdigest()
    payload["corrections"] = {
        "supersedes_run_id": "run-original",
        "supersedes_attempt_id": "attempt-original",
        "reason": "synthetic correction test",
    }
    artifact_id = "original-record" if include_original else "missing-original"
    if include_original:
        add_artifact(payload, "original.json", original_bytes,
                     "original_record", artifact_id)
        (root / "original.json").write_bytes(original_bytes)
    payload["prior_evidence_refs"] = [{
        "kind": "run", "path": "original.json", "attempt_id": "attempt-original",
        "manifest_id": None, "run_id": "run-original", "sha256": digest,
        "artifact_id": artifact_id, "relation": "supersedes",
        "provides_measurements_for_this_run": False,
    }]


def rewrite_json_artifact(
    payload: dict[str, object],
    root: Path,
    role: str,
    record: dict[str, object],
    artifact_id: str | None = None,
) -> str:
    artifacts = payload["artifacts"]
    assert isinstance(artifacts, list)
    matches = [
        item for item in artifacts
        if item["role"] == role
        and (artifact_id is None or item["artifact_id"] == artifact_id)
    ]
    if len(matches) != 1:
        raise AssertionError(f"expected one {role} fixture artifact")
    descriptor = matches[0]
    content = _canonical_json(record)
    (root / descriptor["path"]).write_bytes(content)
    digest = hashlib.sha256(content).hexdigest()
    descriptor.update({"sha256": digest, "size_bytes": len(content)})
    return digest

def rewrite_v2_receipt(
    payload: dict[str, object],
    root: Path,
    role: str,
    artifact_id: str,
    recorded_at: str,
) -> None:
    artifacts = payload["artifacts"]
    assert isinstance(artifacts, list)
    descriptor = next(
        item for item in artifacts
        if item["role"] == role and item["artifact_id"] == artifact_id
    )
    record = json.loads((root / descriptor["path"]).read_text(encoding="utf-8"))
    record["recorded_at"] = recorded_at
    digest = rewrite_json_artifact(payload, root, role, record, artifact_id)
    lifecycle_key = "start_receipt" if role == "start_receipt" else "terminal_receipt"
    lifecycle_receipt = payload["lifecycle"][lifecycle_key]
    if isinstance(lifecycle_receipt, dict) and lifecycle_receipt.get("receipt_id") == artifact_id:
        lifecycle_receipt["recorded_at"] = recorded_at
        lifecycle_receipt["digest"] = digest


def rewrite_v2_trace_policy(
    payload: dict[str, object],
    root: Path,
    origin: str,
    product_telemetry_opt_in: bool,
) -> None:
    policy = payload["trace_policy"]
    assert isinstance(policy, dict)
    policy["origin"] = origin
    policy["product_telemetry_opt_in"] = product_telemetry_opt_in
    record = json.loads((root / "access-record.json").read_text(encoding="utf-8"))
    record["trace_policy"] = {
        key: value for key, value in policy.items() if key != "artifact_id"
    }
    rewrite_json_artifact(payload, root, "access_record", record)


class BenchmarkEvidenceManifestTests(unittest.TestCase):
    def test_checked_in_manifest_is_valid(self) -> None:
        self.assertEqual(EVIDENCE.errors_for_manifest(MANIFEST), [])

    def test_source_hash_drift_is_rejected(self) -> None:
        payload = json.loads(MANIFEST.read_text(encoding="utf-8"))
        payload["benchmarks"][0]["corpus"]["source_hash"] = "0" * 64
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "manifest.json"
            path.write_text(json.dumps(payload), encoding="utf-8")
            errors = EVIDENCE.errors_for_manifest(path)
        self.assertTrue(any("source_hash" in error for error in errors))

    def test_repository_report_contract_rejects_missing_measurement_status(self) -> None:
        report = {
            "measurement_kind": "real_repository_code_index",
            "evaluation_date": "2026-07-20",
            "corpus_id": "corpus-v1",
            "repository_revision": "commit-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "observations": [{"case_id": "case-1", "route": "PhaseC", "latency_ms": 1}],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "repository.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "repository")
        self.assertTrue(any("measurement_status" in error for error in errors))

    def test_visual_provider_report_binds_to_manifest_corpus(self) -> None:
        manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
        entry = next(
            benchmark
            for benchmark in manifest["benchmarks"]
            if benchmark["benchmark"].startswith("v0.8")
        )
        report = json.loads(
            (ROOT / "tests" / "contracts" / "visual_provider_report_v1.json").read_text(
                encoding="utf-8"
            )
        )
        report["corpus_revision"] = "wrong-revision"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "visual.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "visual-provider", entry)
        self.assertTrue(any("corpus_revision is not bound to its manifest" in error for error in errors))

    def test_visual_provider_report_rejects_unavailable_winner(self) -> None:
        report = json.loads(
            (ROOT / "tests" / "contracts" / "visual_provider_report_v1.json").read_text(
                encoding="utf-8"
            )
        )
        report["winning_classes"] = ["Figure"]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "visual.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "visual-provider")
        self.assertTrue(any("unavailable measurements cannot authorize" in error for error in errors))

    def test_visual_provider_report_rejects_malformed_measurement_and_provider(self) -> None:
        report = json.loads(
            (ROOT / "tests" / "contracts" / "visual_provider_report_v1.json").read_text(
                encoding="utf-8"
            )
        )
        report["observations"][0]["latency_ms"] = True
        report["observations"][0]["measurement_status"] = {"Unavailable": {"reason": True}}
        report["observations"][1]["provider_status"] = {"Unavailable": {"reason": "provider down"}}
        report["observations"][0]["provider_config"]["provider"] = "siglip-onnx"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "visual.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "visual-provider")
        self.assertTrue(any("latency_ms must be a non-negative integer" in error for error in errors))
        self.assertTrue(any("measurement_status is invalid" in error for error in errors))
        self.assertTrue(any("visual provider_status is not Available" in error for error in errors))
        self.assertTrue(any("provider_config is invalid" in error for error in errors))

    def test_visual_fallback_report_requires_measurement_status(self) -> None:
        report = {
            "measurement_kind": "real_visual_provider_unavailable",
            "evaluation_date": "2026-07-20",
            "provider_status": "unavailable",
            "observations": [{"case_id": "case-1", "route": "Visual"}],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "visual-unavailable.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "visual")
        self.assertTrue(any("measurement_status" in error for error in errors))

    def test_build_latency_report_contract_rejects_bad_percentiles(self) -> None:
        report = {
            "measurement_kind": "repository_build_latency",
            "evaluation_date": "2026-08-07",
            "corpus_id": "corpus-v1",
            "repository_revision": "commit-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "sizes": [
                {
                    "files": 50,
                    "symbols": 120,
                    "runs": 5,
                    "p50_ms": 800,
                    "p95_ms": "slow",
                    "measurements_ms": [700, 800, 900],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build-latency.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "build-latency")
        self.assertTrue(any("p95_ms" in error for error in errors))

    def test_build_latency_report_contract_rejects_empty_measurements(self) -> None:
        report = {
            "measurement_kind": "repository_build_latency",
            "evaluation_date": "2026-08-07",
            "corpus_id": "corpus-v1",
            "repository_revision": "commit-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "sizes": [{"files": 50, "symbols": 120, "runs": 0, "p50_ms": 0, "p95_ms": 0}],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build-latency.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "build-latency")
        self.assertTrue(any("measurements_ms" in error for error in errors))

    def test_build_latency_report_contract_accepts_well_formed_report(self) -> None:
        report = {
            "measurement_kind": "repository_build_latency",
            "evaluation_date": "2026-08-07",
            "corpus_id": "corpus-v1",
            "repository_revision": "commit-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "sizes": [
                {
                    "files": 50,
                    "symbols": 120,
                    "runs": 5,
                    "p50_ms": 800,
                    "p95_ms": 900,
                    "measurements_ms": [700, 800, 800, 900, 950],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "build-latency.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "build-latency")
        self.assertEqual(errors, [])

    def test_learned_sparse_report_contract_accepts_well_formed_report(self) -> None:
        report = {
            "measurement_kind": "learned_sparse_four_profile",
            "evaluation_date": "2026-08-07",
            "corpus_id": "corpus-v1",
            "corpus_revision": "rev-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "namespace": "instance-a:verified:sparse_text_v1",
            "route_configuration": {"route": "SparseFused", "result_limit": 20},
            "observations": [
                {
                    "case_id": "case-1",
                    "route": "SparseFused",
                    "quality": {"recall_at_5": 1},
                    "resources": {"p50_latency_ms": 12},
                    "safety": {"acl_leakage": 0},
                    "measurement_status": "Measured",
                }
            ],
            "decisions": {
                "ExactLiteral": "RetainLexical",
                "VocabularyExpansion": "PromoteSparseFused",
                "DomainTerminology": "RetainHybrid",
                "MultiTerm": "RetainHybrid",
                "NoEvidence": "RetainLexical",
                "Security": "RetainLexical",
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "learned-sparse.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "learned-sparse")
        self.assertEqual(errors, [])

    def test_learned_sparse_report_contract_rejects_bad_route_and_status(self) -> None:
        report = {
            "measurement_kind": "learned_sparse_four_profile",
            "evaluation_date": "2026-08-07",
            "corpus_id": "corpus-v1",
            "corpus_revision": "rev-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "namespace": "instance-a:verified:sparse_text_v1",
            "route_configuration": {"route": "Hybrid"},
            "observations": [
                {
                    "case_id": "case-1",
                    "route": "Vector",
                    "quality": {},
                    "resources": {},
                    "safety": {},
                    "measurement_status": {"Unavailable": {"reason": ""}},
                }
            ],
            "decisions": {"VocabularyExpansion": "PromoteSparseFused"},
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "learned-sparse.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "learned-sparse")
        self.assertTrue(any("route_configuration" in error for error in errors))
        self.assertTrue(any("route is invalid" in error for error in errors))
        self.assertTrue(any("measurement_status" in error for error in errors))
        self.assertTrue(any("decisions[ExactLiteral]" in error for error in errors))

    def test_learned_sparse_report_contract_rejects_protected_promotion(self) -> None:
        report = {
            "measurement_kind": "learned_sparse_four_profile",
            "evaluation_date": "2026-08-07",
            "corpus_id": "corpus-v1",
            "corpus_revision": "rev-v1",
            "index_generation": "index-v1",
            "model_fingerprint": "model-v1",
            "namespace": "instance-a:verified:sparse_text_v1",
            "route_configuration": {"route": "SparseFused", "result_limit": 20},
            "observations": [
                {
                    "case_id": "case-1",
                    "route": "Lexical",
                    "quality": {"recall_at_5": 1},
                    "resources": {"p50_latency_ms": 12},
                    "safety": {"acl_leakage": 0},
                    "measurement_status": "Measured",
                }
            ],
            "decisions": {
                "ExactLiteral": "PromoteSparseFused",
                "VocabularyExpansion": "RetainHybrid",
                "DomainTerminology": "RetainHybrid",
                "MultiTerm": "RetainHybrid",
                "NoEvidence": "RetainLexical",
                "Security": "RetainLexical",
            },
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "learned-sparse.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            errors = EVIDENCE.errors_for_report(path, "learned-sparse")
        self.assertTrue(any("protected classes cannot be promoted" in error for error in errors))




class BenchmarkRunEvidenceV2Tests(unittest.TestCase):
    def write_manifest(self, directory: Path, payload: dict[str, object]) -> Path:
        path = directory / "run.json"
        path.write_text(json.dumps(payload), encoding="utf-8")
        return path

    def test_v1_cli_dispatch_preserves_the_checked_in_ledger(self) -> None:
        with redirect_stdout(io.StringIO()):
            self.assertEqual(EVIDENCE.validate(MANIFEST, None), 0)

    def test_unknown_schema_version_is_rejected_by_cli(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "unknown.json"
            path.write_text(json.dumps({"schema_version": 9}), encoding="utf-8")
            output = io.StringIO()
            with redirect_stdout(output):
                result = EVIDENCE.validate(path, None)
        self.assertEqual(result, 1)

    def test_noninteger_schema_versions_do_not_dispatch_as_v1(self) -> None:
        for version in (True, 1.0, 2.0):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "wrong-version.json"
                path.write_text(json.dumps({"schema_version": version}), encoding="utf-8")
                output = io.StringIO()
                with redirect_stdout(output):
                    result = EVIDENCE.validate(path, None)
                self.assertEqual(result, 1)

    def test_deeply_nested_json_is_rejected_by_cli_without_traceback(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "deeply-nested.json"
            nested = "[" * 3000 + "0" + "]" * 3000
            path.write_text('{"schema_version":' + nested + "}", encoding="utf-8")
            output = io.StringIO()
            with redirect_stdout(output):
                result = EVIDENCE.validate(path, None)
        self.assertEqual(result, 1)
        self.assertNotIn("Traceback", output.getvalue())

    def test_verified_synthetic_evidence_chain_supports_eligibility(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            path = self.write_manifest(base, payload)
            output = io.StringIO()
            with redirect_stdout(output):
                result = EVIDENCE.validate(path, None, root)
        self.assertEqual(result, 0)

    def test_empty_evidence_cannot_qualify_from_manifest_echoes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            descriptor = next(item for item in payload["artifacts"]
                              if item["role"] == "observations")
            empty = b"{}"
            (root / descriptor["path"]).write_bytes(empty)
            descriptor["sha256"] = hashlib.sha256(empty).hexdigest()
            descriptor["size_bytes"] = len(empty)
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_failed_first_attempt_and_its_cost_are_retained_in_campaign_ledger(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, include_failed_retry=True)
            path = self.write_manifest(base, payload)
            validator = EVIDENCE._v2_validator()
            self.assertTrue(validator.validate_manifest(path, root).confirmatory_eligible)

            ledger_descriptor = next(item for item in payload["artifacts"]
                                     if item["role"] == "attempt_ledger")
            ledger_path = root / ledger_descriptor["path"]
            ledger = json.loads(ledger_path.read_text(encoding="utf-8"))
            ledger["attempts"].pop()
            ledger_bytes = _canonical_json(ledger)
            ledger_path.write_bytes(ledger_bytes)
            ledger_descriptor.update({
                "sha256": hashlib.sha256(ledger_bytes).hexdigest(),
                "size_bytes": len(ledger_bytes),
            })
            observations_descriptor = next(item for item in payload["artifacts"]
                                           if item["role"] == "observations")
            observations_path = root / observations_descriptor["path"]
            observations = json.loads(observations_path.read_text(encoding="utf-8"))
            observations["attempt_ledger_digest"] = ledger_descriptor["sha256"]
            observations_bytes = _canonical_json(observations)
            observations_path.write_bytes(observations_bytes)
            observations_descriptor.update({
                "sha256": hashlib.sha256(observations_bytes).hexdigest(),
                "size_bytes": len(observations_bytes),
            })
            path = self.write_manifest(base, payload)
            result = validator.validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.errors)

    def test_campaign_consumption_records_are_included_in_actual_totals(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, include_failed_retry=True)
            ledger_descriptor = next(
                item for item in payload["artifacts"] if item["role"] == "attempt_ledger"
            )
            ledger = json.loads((root / ledger_descriptor["path"]).read_text(encoding="utf-8"))
            rows = [*ledger["attempts"], *ledger["consumptions"]]
            compute_total = sum(
                (Decimal(str(row["compute_seconds"])) for row in rows), Decimal(0)
            )
            cost_total = sum(
                (Decimal(str(row["cost_amount"])) for row in rows), Decimal(0)
            )
            phases = {row["phase"] for row in ledger["consumptions"]}
            setup = [row for row in ledger["consumptions"] if row["phase"] == "setup"]
            preparation = [row for row in ledger["consumptions"]
                           if row["phase"] == "preparation"]
            self.assertEqual(phases, {"preparation", "tuning", "setup"})
            self.assertEqual([row["state"] for row in setup], ["failed", "complete"])
            self.assertEqual(Decimal(str(payload["budgets"]["actual_compute_seconds"])), compute_total)
            self.assertEqual(Decimal(str(payload["budgets"]["actual_cost_amount"])), cost_total)
            preparation_measure = next(
                item for item in payload["measurements"] if item["name"] == "preparation_wall_ms"
            )
            self.assertEqual(preparation_measure["value"], sum(
                row["wall_time_ms"] for row in preparation
            ))
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertTrue(result.confirmatory_eligible)

    def test_failed_first_evaluation_outcome_is_not_replaced_by_retry(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, include_failed_retry=True)
            ledger = json.loads(
                (root / "attempt-ledger.json").read_text(encoding="utf-8")
            )
            baseline_first, baseline_retry = ledger["attempts"][0], ledger["attempts"][-1]
            observations = json.loads(
                (root / "observations.json").read_text(encoding="utf-8")
            )
            baseline_measure = next(
                item for item in payload["measurements"]
                if item["name"] == "ndcg_at_10_baseline"
            )
            paired_measure = next(
                item for item in payload["measurements"]
                if item["name"] == "ndcg_at_10_paired_difference"
            )
            baseline_observation = next(
                item for item in observations["measurements"]
                if item["name"] == "ndcg_at_10_baseline"
            )["observations"][0]
            self.assertEqual((baseline_first["attempt_number"], baseline_first["state"]), (1, "failed"))
            self.assertEqual((baseline_retry["attempt_number"], baseline_retry["state"]), (2, "complete"))
            self.assertEqual(baseline_observation["state"], "failed")
            self.assertEqual(baseline_observation["value"], 0.0)
            self.assertEqual(baseline_measure["value"], 0.0)
            self.assertEqual(paired_measure["value"], 0.6)
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertTrue(result.confirmatory_eligible)

    def test_successful_retry_cannot_replace_frozen_first_attempt_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, include_failed_retry=True)
            ledger = json.loads((root / "attempt-ledger.json").read_text(encoding="utf-8"))
            failed_first, candidate_first, successful_retry = ledger["attempts"]
            rewrite_v2_receipt(
                payload, root, "start_receipt",
                candidate_first["start_receipt_artifact_id"], "2026-10-07T10:10:00Z",
            )
            rewrite_v2_receipt(
                payload, root, "terminal_receipt",
                candidate_first["terminal_receipt_artifact_id"], "2026-10-07T10:10:03Z",
            )
            payload["lifecycle"]["started_at"] = "2026-10-07T10:10:01Z"
            payload["lifecycle"]["finished_at"] = "2026-10-07T10:10:02Z"
            positive_path = self.write_manifest(base, payload)
            with redirect_stdout(io.StringIO()):
                positive_status = EVIDENCE.validate(positive_path, None, root)
            self.assertEqual(positive_status, 0)
            frozen_protocol_bytes = (root / "protocol.json").read_bytes()
            successful_receipt_ids = {
                candidate_first["start_receipt_artifact_id"],
                candidate_first["terminal_receipt_artifact_id"],
                successful_retry["start_receipt_artifact_id"],
                successful_retry["terminal_receipt_artifact_id"],
            }
            successful_receipt_bytes = {
                item["artifact_id"]: (root / item["path"]).read_bytes()
                for item in payload["artifacts"]
                if item["artifact_id"] in successful_receipt_ids
            }
            successful_retry["ordinal"] = 1
            successful_retry["attempt_number"] = 1
            candidate_first["ordinal"] = 2
            ledger["attempts"] = [successful_retry, candidate_first]
            removed_receipts = {
                failed_first["start_receipt_artifact_id"],
                failed_first["terminal_receipt_artifact_id"],
            }
            payload["artifacts"] = [
                item for item in payload["artifacts"]
                if item["artifact_id"] not in removed_receipts
            ]
            rows = [*ledger["attempts"], *ledger["consumptions"]]
            compute_total = sum(
                (Decimal(str(row["compute_seconds"])) for row in rows), Decimal(0)
            )
            cost_total = sum(
                (Decimal(str(row["cost_amount"])) for row in rows), Decimal(0)
            )
            ledger["total_compute_seconds"] = float(compute_total)
            ledger["total_cost_amount"] = float(cost_total)
            payload["budgets"]["actual_compute_seconds"] = float(compute_total)
            payload["budgets"]["actual_cost_amount"] = float(cost_total)
            ledger_digest = rewrite_json_artifact(payload, root, "attempt_ledger", ledger)
            observations = json.loads((root / "observations.json").read_text(encoding="utf-8"))
            observations["attempt_ledger_digest"] = ledger_digest
            candidate_quality = next(
                measure["observations"][0]["value"]
                for measure in observations["measurements"]
                if measure["name"] == "ndcg_at_10_candidate"
            )
            baseline_quality = 0.4
            paired_difference = float(
                Decimal(str(candidate_quality)) - Decimal(str(baseline_quality))
            )
            for measure in observations["measurements"]:
                if measure["name"].endswith("_baseline"):
                    measure["observations"][0].update({
                        "run_id": successful_retry["run_id"],
                        "attempt_id": successful_retry["attempt_id"],
                        "state": successful_retry["state"],
                    })
                    if measure["name"] == "ndcg_at_10_baseline":
                        measure["observations"][0]["value"] = baseline_quality
                elif measure["name"] == "ndcg_at_10_paired_difference":
                    measure["observations"][0]["value"] = paired_difference
            baseline_measure = next(
                item for item in payload["measurements"]
                if item["name"] == "ndcg_at_10_baseline"
            )
            paired_measure = next(
                item for item in payload["measurements"]
                if item["name"] == "ndcg_at_10_paired_difference"
            )
            baseline_measure["value"] = baseline_quality
            paired_measure["value"] = paired_difference
            path = self.write_manifest(base, payload)
            output = io.StringIO()
            with redirect_stdout(output):
                public_status = EVIDENCE.validate(path, None, root)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
            self.assertEqual((root / "protocol.json").read_bytes(), frozen_protocol_bytes)
            for item in payload["artifacts"]:
                if item["artifact_id"] in successful_receipt_ids:
                    self.assertEqual(
                        (root / item["path"]).read_bytes(),
                        successful_receipt_bytes[item["artifact_id"]],
                    )
        self.assertEqual(public_status, 1)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_frozen_first_attempt_order_matches_receipt_chronology(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            ledger = json.loads((root / "attempt-ledger.json").read_text(encoding="utf-8"))
            candidate = ledger["attempts"][1]
            rewrite_v2_receipt(
                payload, root, "start_receipt", candidate["start_receipt_artifact_id"],
                "2026-10-07T08:59:50Z",
            )
            rewrite_v2_receipt(
                payload, root, "terminal_receipt", candidate["terminal_receipt_artifact_id"],
                "2026-10-07T08:59:55Z",
            )
            payload["lifecycle"]["started_at"] = "2026-10-07T08:59:51Z"
            payload["lifecycle"]["finished_at"] = "2026-10-07T08:59:54Z"
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_overlapping_attempts_require_frozen_protocol_permission(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, max_concurrent_attempts=1)
            ledger = json.loads((root / "attempt-ledger.json").read_text(encoding="utf-8"))
            candidate = ledger["attempts"][1]
            rewrite_v2_receipt(
                payload, root, "start_receipt", candidate["start_receipt_artifact_id"],
                "2026-10-07T09:00:01Z",
            )
            rewrite_v2_receipt(
                payload, root, "terminal_receipt", candidate["terminal_receipt_artifact_id"],
                "2026-10-07T09:00:03Z",
            )
            payload["lifecycle"]["started_at"] = "2026-10-07T09:00:01Z"
            payload["lifecycle"]["finished_at"] = "2026-10-07T09:00:02Z"
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_explicit_protocol_concurrency_permits_overlapping_attempts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, max_concurrent_attempts=2)
            ledger = json.loads((root / "attempt-ledger.json").read_text(encoding="utf-8"))
            candidate = ledger["attempts"][1]
            rewrite_v2_receipt(
                payload, root, "start_receipt", candidate["start_receipt_artifact_id"],
                "2026-10-07T09:00:01Z",
            )
            rewrite_v2_receipt(
                payload, root, "terminal_receipt", candidate["terminal_receipt_artifact_id"],
                "2026-10-07T09:00:03Z",
            )
            payload["lifecycle"]["started_at"] = "2026-10-07T09:00:01Z"
            payload["lifecycle"]["finished_at"] = "2026-10-07T09:00:02Z"
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertTrue(result.confirmatory_eligible)

    def test_retry_must_start_after_its_predecessor_terminal_receipt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(
                root, include_failed_retry=True, max_concurrent_attempts=3
            )
            ledger = json.loads((root / "attempt-ledger.json").read_text(encoding="utf-8"))
            retry = ledger["attempts"][-1]
            rewrite_v2_receipt(
                payload, root, "start_receipt", retry["start_receipt_artifact_id"],
                "2026-10-07T09:00:01Z",
            )
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_frozen_protocol_mutation_does_not_rebind_existing_start_receipts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            protocol = json.loads((root / "protocol.json").read_text(encoding="utf-8"))
            protocol["freeze_id"] = "mutated-freeze"
            digest = rewrite_json_artifact(payload, root, "protocol", protocol)
            payload["protocol"]["digest"] = digest
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_system_configuration_bytes_match_frozen_configuration_digests(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            configuration = json.loads(
                (root / "configuration-baseline.json").read_text(encoding="utf-8")
            )
            configuration["configuration"]["mode"] = "altered"
            rewrite_json_artifact(
                payload, root, "system_configuration", configuration,
                "configuration-baseline",
            )
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_paired_difference_is_recomputed_from_matched_outcomes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            observations = json.loads(
                (root / "observations.json").read_text(encoding="utf-8")
            )
            paired = next(
                item for item in observations["measurements"]
                if item["name"] == "ndcg_at_10_paired_difference"
            )
            paired["observations"][0]["value"] = 0.3
            manifest_pair = next(
                item for item in payload["measurements"]
                if item["name"] == "ndcg_at_10_paired_difference"
            )
            manifest_pair["value"] = 0.3
            rewrite_json_artifact(payload, root, "observations", observations)
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_resource_measurements_obey_frozen_maxima(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            observations = json.loads(
                (root / "observations.json").read_text(encoding="utf-8")
            )
            latency = next(
                item for item in observations["measurements"]
                if item["name"] == "fast_path_p95_latency_ms_candidate"
            )
            latency["observations"][0]["value"] = 101.0
            manifest_latency = next(
                item for item in payload["measurements"]
                if item["name"] == "fast_path_p95_latency_ms_candidate"
            )
            manifest_latency["value"] = 101.0
            rewrite_json_artifact(payload, root, "observations", observations)
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_product_telemetry_accepts_only_matching_affirmative_opt_in(self) -> None:
        for opt_in, expected_status in ((True, 0), (False, 1)):
            with self.subTest(opt_in=opt_in), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                root = base / "artifacts"
                payload = complete_v2_run(root)
                rewrite_v2_trace_policy(payload, root, "product_telemetry", opt_in)
                path = self.write_manifest(base, payload)
                with redirect_stdout(io.StringIO()):
                    status = EVIDENCE.validate(path, None, root)
            self.assertEqual(status, expected_status)

    def test_original_record_schema_version_requires_an_exact_integer(self) -> None:
        for version, expected_status in ((2, 0), (2.0, 1)):
            with self.subTest(version=version), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                root = base / "artifacts"
                payload = complete_v2_run(root)
                add_original_reference(payload, root, include_original=True)
                original = json.loads((root / "original.json").read_text(encoding="utf-8"))
                original["schema_version"] = version
                digest = rewrite_json_artifact(payload, root, "original_record", original)
                payload["prior_evidence_refs"][0]["sha256"] = digest
                path = self.write_manifest(base, payload)
                with redirect_stdout(io.StringIO()):
                    status = EVIDENCE.validate(path, None, root)
            self.assertEqual(status, expected_status)

    def test_public_validation_rejects_unknown_cache_boundaries(self) -> None:
        boundaries = ("process", "index", "model", "application", "os", "remote")
        for boundary in boundaries:
            with self.subTest(boundary=boundary), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                root = base / "artifacts"
                cache = {
                    "process": "fresh", "index": "fresh", "model": "warm",
                    "application": "fresh", "os": "cold", "remote": "unused",
                }
                cache[boundary] = "unknown"
                payload = complete_v2_run(root, cache_state=cache)
                path = self.write_manifest(base, payload)
                output = io.StringIO()
                with redirect_stdout(output):
                    status = EVIDENCE.validate(path, None, root)
            self.assertEqual(status, 1)

    def test_public_validation_rejects_zero_ram(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, ram_bytes=0)
            path = self.write_manifest(base, payload)
            output = io.StringIO()
            with redirect_stdout(output):
                status = EVIDENCE.validate(path, None, root)
        self.assertEqual(status, 1)

    def test_boolean_evidence_version_is_rejected_as_noninteger(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            protocol = json.loads((root / "protocol.json").read_text(encoding="utf-8"))
            protocol["evidence_version"] = True
            rewrite_json_artifact(payload, root, "protocol", protocol)
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_correction_requires_verified_original_record_bytes(self) -> None:
        validator = EVIDENCE._v2_validator()
        for include_original in (True, False):
            with self.subTest(include_original=include_original), tempfile.TemporaryDirectory() as directory:
                base = Path(directory)
                root = base / "artifacts"
                payload = complete_v2_run(root)
                add_original_reference(payload, root, include_original)
                path = self.write_manifest(base, payload)
                result = validator.validate_manifest(path, root)
            self.assertEqual(result.confirmatory_eligible, include_original)

    def test_correction_reference_digest_must_match_original_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            add_original_reference(payload, root, include_original=True)
            payload["prior_evidence_refs"][0]["sha256"] = "0" * 64
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)

    def test_access_record_must_bind_declared_consent(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            descriptor = next(item for item in payload["artifacts"]
                              if item["role"] == "access_record")
            record_path = root / descriptor["path"]
            record = json.loads(record_path.read_text(encoding="utf-8"))
            record["trace_policy"]["product_telemetry_opt_in"] = True
            content = _canonical_json(record)
            record_path.write_bytes(content)
            descriptor.update({
                "sha256": hashlib.sha256(content).hexdigest(), "size_bytes": len(content)
            })
            path = self.write_manifest(base, payload)
            result = EVIDENCE._v2_validator().validate_manifest(path, root)
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.artifact_verification_complete)

    def test_malformed_shapes_paths_json_and_utf8_fail_closed(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            malformed = v2_proposed_manifest()
            malformed["artifacts"] = None
            result = validator.validate_manifest(self.write_manifest(base, malformed))
            self.assertTrue(result.errors)

            bad_path = v2_proposed_manifest()
            add_artifact(bad_path, "bad-\ud800.json", b"{}")
            result = validator.validate_manifest(self.write_manifest(base, bad_path))
            self.assertTrue(result.errors)

            root = base / "artifacts"
            root.mkdir()
            for name, raw, media in (
                ("malformed.json", b'{"evidence":', "application/json"),
                ("invalid.txt", b"x" * 8192 + b"\xff", "text/plain"),
                ("wrong-media.json", b"{}", "text/plain"),
            ):
                with self.subTest(name=name):
                    (root / name).write_bytes(raw)
                    payload = v2_proposed_manifest()
                    add_artifact(payload, name, raw, media_type=media)
                    result = validator.validate_manifest(self.write_manifest(base, payload), root)
                    self.assertTrue(result.errors)


    def test_v2_cli_dispatch_verifies_only_with_explicit_artifact_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"
            root.mkdir()
            (root / "observations.json").write_bytes(b"{}")
            payload = v2_proposed_manifest()
            add_artifact(payload, "observations.json", b"{}")
            path = self.write_manifest(Path(directory), payload)
            with redirect_stdout(io.StringIO()):
                status = EVIDENCE.validate(path, None, root)
            validator = EVIDENCE._v2_validator()
            verified = validator.validate_manifest(path, root)
            unverified = validator.validate_manifest(path)
        self.assertEqual(status, 0)
        self.assertTrue(verified.artifact_verification_complete)
        self.assertFalse(verified.confirmatory_eligible)
        self.assertFalse(unverified.artifact_verification_complete)


    def test_proposed_manifest_is_structural_only(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            path = self.write_manifest(Path(directory), v2_proposed_manifest())
            result = validator.validate_manifest(path)
        self.assertEqual(result.errors, [])
        self.assertFalse(result.confirmatory_eligible)
        self.assertTrue(result.warnings)

    def test_proposed_or_not_run_cannot_claim_eligibility(self) -> None:
        validator = EVIDENCE._v2_validator()
        for state in ("proposed", "not_run"):
            with self.subTest(state=state), tempfile.TemporaryDirectory() as directory:
                payload = v2_proposed_manifest()
                payload["state"] = state
                claim = payload["claim_eligibility"]
                assert isinstance(claim, dict)
                claim["status"] = "eligible"
                path = self.write_manifest(Path(directory), payload)
                result = validator.validate_manifest(path)
            self.assertFalse(result.confirmatory_eligible)
            self.assertTrue(result.errors)

    def test_unavailable_measurement_rejects_zero_but_measured_zero_is_valid(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "unavailable.json"
            payload = v2_proposed_manifest()
            measurement = payload["measurements"][0]
            measurement["value"] = 0
            measurement["status"] = "unavailable"
            result = validator.validate_manifest(self.write_manifest(Path(directory), payload))
            self.assertTrue(result.errors)

            measured = v2_proposed_manifest()
            artifact_bytes = b"{}"
            add_artifact(measured, "observations.json", artifact_bytes)
            measured["data"]["timing_repetitions"] = 1
            observed = measured["measurements"][0]
            observed.update(
                {
                    "value": 0,
                    "status": "measured",
                    "method": "predeclared calculation",
                    "denominator": {"kind": "timing_repetitions", "count": 1},
                    "observations_artifact": "artifact-1",
                }
            )
            result = validator.validate_manifest(self.write_manifest(Path(directory), measured))
        self.assertEqual(result.errors, [])

    def test_boolean_and_non_finite_measurements_are_rejected(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            payload = v2_proposed_manifest()
            payload["measurements"][0]["value"] = True
            boolean_result = validator.validate_manifest(self.write_manifest(Path(directory), payload))
            self.assertTrue(boolean_result.errors)
            for non_finite in (float("nan"), float("inf"), float("-inf")):
                with self.subTest(non_finite=str(non_finite)):
                    payload = v2_proposed_manifest()
                    payload["measurements"][0]["value"] = non_finite
                    result = validator.validate_manifest(self.write_manifest(Path(directory), payload))
                    self.assertTrue(result.errors)


    def test_terminal_state_without_terminal_receipt_is_not_complete(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            payload = v2_proposed_manifest()
            payload["state"] = "complete"
            payload["attempt_id"] = "attempt-1"
            payload["lifecycle"].update(
                {
                    "start_receipt": {
                        "receipt_id": "start-1",
                        "attempt_id": "attempt-1",
                        "run_id": "proposal-1",
                        "protocol_id": "protocol-1",
                        "protocol_revision": "draft",
                        "recorded_at": "2026-10-07T10:00:00Z",
                        "digest": "a" * 64,
                    },
                    "started_at": "2026-10-07T10:00:01Z",
                }
            )
            result = validator.validate_manifest(self.write_manifest(Path(directory), payload))
        self.assertTrue(result.errors)
        self.assertFalse(result.confirmatory_eligible)

    def test_missing_and_altered_artifact_bytes_are_rejected(self) -> None:
        validator = EVIDENCE._v2_validator()
        expected = b"{}"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"
            root.mkdir()
            missing = v2_proposed_manifest()
            add_artifact(missing, "observations.json", expected)
            result = validator.validate_manifest(
                self.write_manifest(Path(directory), missing), root
            )
            self.assertTrue(result.errors)

            (root / "observations.json").write_bytes(b"[]")
            altered = v2_proposed_manifest()
            add_artifact(altered, "observations.json", expected)
            result = validator.validate_manifest(
                self.write_manifest(Path(directory), altered), root
            )
        self.assertTrue(result.errors)
        self.assertNotIn(hashlib.sha256(expected).hexdigest(), " ".join(result.errors))

    def test_artifact_size_and_media_type_are_checked(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"
            root.mkdir()
            (root / "observations.json").write_bytes(b"{ }")
            size_mismatch = v2_proposed_manifest()
            add_artifact(size_mismatch, "observations.json", b"{}")
            result = validator.validate_manifest(
                self.write_manifest(Path(directory), size_mismatch), root
            )
            self.assertTrue(result.errors)

            (root / "observations.json").write_bytes(b"nope")
            media_mismatch = v2_proposed_manifest()
            add_artifact(media_mismatch, "observations.json", b"nope", media_type="text/plain")
            result = validator.validate_manifest(
                self.write_manifest(Path(directory), media_mismatch), root
            )
        self.assertTrue(result.errors)

    def test_inaccessible_private_artifact_remains_unverified_and_ineligible(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "artifacts"
            root.mkdir()
            payload = v2_proposed_manifest()
            add_artifact(payload, "private.json", b"{}")
            payload["artifacts"][0]["access"] = {
                "classification": "private",
                "reason": "Restricted source data.",
            }
            result = validator.validate_manifest(
                self.write_manifest(Path(directory), payload), root
            )
        self.assertEqual(result.errors, [])
        self.assertFalse(result.confirmatory_eligible)
        self.assertFalse(result.artifact_verification_complete)

    def test_artifact_traversal_and_symlink_chain_are_rejected(self) -> None:
        validator = EVIDENCE._v2_validator()
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent / "artifacts"
            root.mkdir()
            traversal = v2_proposed_manifest()
            add_artifact(traversal, "../outside.json", b"{}")
            result = validator.validate_manifest(self.write_manifest(parent, traversal), root)
            self.assertTrue(result.errors)

            outside = parent / "outside"
            outside.mkdir()
            (outside / "evidence.json").write_bytes(b"{}")
            try:
                (root / "linked").symlink_to(outside, target_is_directory=True)
            except OSError:
                self.skipTest("symlink creation is unavailable in this environment")
            chained = v2_proposed_manifest()
            add_artifact(chained, "linked/evidence.json", b"{}")
            result = validator.validate_manifest(self.write_manifest(parent, chained), root)
        self.assertTrue(result.errors)

    def test_reference_view_redacts_private_paths_and_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            private = payload["artifacts"][0]
            private["access"] = {
                "classification": "private",
                "reason": "synthetic restricted evidence",
            }
            payload["prior_evidence_refs"].append({
                "kind": "run",
                "path": "/private/query-title/manifest.json",
                "attempt_id": "historical-attempt",
                "manifest_id": "historical-manifest",
                "run_id": "historical-run",
                "sha256": "f" * 64,
                "artifact_id": None,
                "relation": "historical",
                "provides_measurements_for_this_run": False,
            })
            path = self.write_manifest(base, payload)
            original_bytes = path.read_bytes()
            original_artifacts = {
                item["path"]: (root / item["path"]).read_bytes()
                for item in payload["artifacts"]
            }
            output = io.StringIO()
            with redirect_stdout(output):
                status = EVIDENCE.validate(path, None, root, "references")
            view = json.loads(output.getvalue())
            serialized = output.getvalue()
            self.assertEqual(path.read_bytes(), original_bytes)
            self.assertEqual(
                {
                    artifact_path: (root / artifact_path).read_bytes()
                    for artifact_path in original_artifacts
                },
                original_artifacts,
            )
        private_view = next(
            item for item in view["artifacts"] if item["artifact_id"] == private["artifact_id"]
        )
        historical = view["references"][0]
        self.assertEqual(status, 0)
        self.assertEqual(private_view["status"], "verified")
        self.assertEqual(private_view["access"], "private")
        self.assertIsNone(private_view["path"])
        self.assertEqual(historical["status"], "unverified")
        self.assertIsNone(historical["path"])
        self.assertNotIn("/private/query-title/manifest.json", serialized)
        self.assertNotIn("f" * 64, serialized)
        self.assertNotIn("sha256", serialized)
        self.assertNotIn(str(root), serialized)

    def test_claim_view_reports_first_failure_without_replacing_it_with_retry(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root, include_failed_retry=True)
            path = self.write_manifest(base, payload)
            original_bytes = path.read_bytes()
            original_artifacts = {
                item["path"]: (root / item["path"]).read_bytes()
                for item in payload["artifacts"]
            }
            output = io.StringIO()
            with redirect_stdout(output):
                status = EVIDENCE.validate(path, None, root, "claims")
            view = json.loads(output.getvalue())
            self.assertEqual(path.read_bytes(), original_bytes)
            self.assertEqual(
                {
                    artifact_path: (root / artifact_path).read_bytes()
                    for artifact_path in original_artifacts
                },
                original_artifacts,
            )
        baseline = next(
            item for item in view["measurements"] if item["name"] == "ndcg_at_10_baseline"
        )
        baseline_first = next(
            item for item in view["first_attempts"] if item["system_id"] == "baseline"
        )
        retry = next(
            item for item in view["attempts"]
            if item["system_id"] == "baseline" and item["attempt_number"] == 2
        )
        first_quality = next(
            item for item in baseline_first["measurements"]
            if item["name"] == "ndcg_at_10_baseline"
        )
        self.assertEqual(status, 0)
        self.assertTrue(view["derived"])
        self.assertTrue(view["read_only"])
        self.assertTrue(view["qualification"]["eligible"])
        self.assertEqual(baseline["value"], 0.0)
        self.assertEqual(baseline["denominator"], {
            "kind": "independent_needs",
            "count": 1,
        })
        self.assertEqual(baseline_first["state"], "failed")
        self.assertEqual(first_quality["value"], 0.0)
        self.assertEqual(retry["state"], "complete")

    def test_unverified_measurement_reference_cannot_qualify_claim(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            payload["prior_evidence_refs"].append({
                "kind": "run",
                "path": "external/prior-run.json",
                "attempt_id": "historical-attempt",
                "manifest_id": "historical-manifest",
                "run_id": "historical-run",
                "sha256": "e" * 64,
                "artifact_id": None,
                "relation": "measured_input",
                "provides_measurements_for_this_run": True,
            })
            path = self.write_manifest(base, payload)
            output = io.StringIO()
            with redirect_stdout(output):
                status = EVIDENCE.validate(path, None, root, "claims")
            view = json.loads(output.getvalue())
        reference = view["reference_index"]["references"][0]
        estimate = next(
            item for item in view["measurements"]
            if item["name"] == "ndcg_at_10_paired_difference"
        )
        self.assertEqual(status, 1)
        self.assertFalse(view["qualification"]["eligible"])
        self.assertEqual(view["qualification"]["status"], "blocked")
        self.assertIn(
            "measurement_reference_unverified",
            view["qualification"]["diagnostics"],
        )
        self.assertEqual(reference["status"], "unverified")
        self.assertIsNone(reference["path"])
        self.assertEqual(estimate["status"], "unverified")
        self.assertEqual(estimate["reason"], "measurement_reference_unverified")
        self.assertIsNone(estimate["value"])

    def test_nonqualifying_claim_view_keeps_verified_measurements_visible(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            payload["claim_eligibility"]["status"] = "ineligible"
            payload["claim_eligibility"]["claim_ids"] = []
            path = self.write_manifest(base, payload)
            output = io.StringIO()
            with redirect_stdout(output):
                status = EVIDENCE.validate(path, None, root, "claims")
            view = json.loads(output.getvalue())
        estimate = next(
            item for item in view["measurements"]
            if item["name"] == "ndcg_at_10_paired_difference"
        )
        self.assertEqual(status, 1)
        self.assertFalse(view["qualification"]["eligible"])
        self.assertEqual(view["qualification"]["status"], "ineligible")
        self.assertEqual(view["qualification"]["claim_ids"], [])
        self.assertEqual(estimate["status"], "measured")
        self.assertEqual(estimate["value"], 0.2)

    def test_claim_view_without_observations_blocks_numeric_claims(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            root = base / "artifacts"
            payload = complete_v2_run(root)
            descriptor = next(item for item in payload["artifacts"] if item["role"] == "observations")
            (root / descriptor["path"]).unlink()
            path = self.write_manifest(base, payload)
            output = io.StringIO()
            with redirect_stdout(output):
                status = EVIDENCE.validate(path, None, root, "claims")
            view = json.loads(output.getvalue())
        observation = next(
            item for item in view["reference_index"]["artifacts"]
            if item["role"] == "observations"
        )
        self.assertEqual(status, 1)
        self.assertFalse(view["qualification"]["eligible"])
        self.assertEqual(view["qualification"]["status"], "blocked")
        self.assertEqual(observation["status"], "invalid")
        self.assertTrue(all(item["value"] is None for item in view["measurements"]))

    def test_views_do_not_attribute_rebound_foreign_execution_to_this_run(self) -> None:
        for declared in ("eligible", "ineligible"):
            for kind in ("references", "claims"):
                with self.subTest(declared=declared, view=kind), tempfile.TemporaryDirectory() as directory:
                    base = Path(directory)
                    root = base / "artifacts"
                    payload = complete_v2_run(root)
                    payload["claim_eligibility"]["status"] = declared
                    if declared == "ineligible":
                        payload["claim_eligibility"]["claim_ids"] = []
                    descriptor = next(item for item in payload["artifacts"] if item["role"] == "execution")
                    execution = json.loads((root / descriptor["path"]).read_text())
                    execution["run_id"] = "unrelated-public-run"
                    rewrite_json_artifact(payload, root, "execution", execution)
                    path = self.write_manifest(base, payload)
                    output = io.StringIO()
                    with redirect_stdout(output):
                        status = EVIDENCE.validate(path, None, root, kind)
                    view = json.loads(output.getvalue())
                self.assertEqual(status, 1)
                if kind == "references":
                    self.assertFalse(view["artifact_verification_complete"])
                    self.assertEqual(view["artifacts"], [])
                    self.assertEqual(view["references"], [])
                else:
                    self.assertEqual(view["qualification"]["status"], "blocked")
                    self.assertFalse(view["qualification"]["eligible"])
                    self.assertEqual(view["first_attempts"], [])
                    self.assertEqual(view["attempts"], [])
                    self.assertEqual(view["activities"], [])
                    self.assertEqual(view["measurements"], [])
                    self.assertIsNone(view["totals"])


if __name__ == "__main__":
    unittest.main()
