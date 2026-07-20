#!/usr/bin/env python3
"""Validate, generate, and reconcile release milestone exit-evidence contracts.

The release workflow consumes a milestone description block so each release can
move through explicit readiness stages before publication.

This module supports:
  - Validating exit evidence payloads against contract rules.
  - Generating machine-readable exit evidence blocks from structured data.
  - Reconciling exit evidence against reference (actual) metrics.
  - Validating post-release follow-up tracking completeness.
  - Environment consistency and artifact-linking for reproducible reports.
  - GoldenProfile support for future milestone profiling.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections.abc import Mapping, Sequence
from datetime import datetime
from pathlib import Path
from typing import Any

RELEASE_STATES = (
    "implementation-complete",
    "benchmark-complete",
    "product-complete",
    "released",
)

RELEASE_STATE_INDEX = {state: index for index, state in enumerate(RELEASE_STATES)}

_REQUIRED_FINGERPRINT_KEYS = (
    "corpus_snapshot",
    "index_generation",
    "model_fingerprint",
)

_REQUIRED_ENVIRONMENT_KEYS = (
    "os",
    "rust_toolchain",
    "cpu_arch",
)

_ALLOWED_ARTIFACT_SOURCES = {"ci", "manual", "external"}

_ALLOWED_PROFILE_STAGES = {
    "baseline",
    "golden",
    "shadow",
    "promoted",
    "retired",
}

_BLOCK_FENCE_PATTERN = re.compile(
    r"(?ms)^```(?:release-exit-evidence|json)\n(.*?)^```$"
)

_ALLOWED_RESULT_STATUS = {"pass", "warning", "fail", "pending", "n/a"}
_ALLOWED_DATA_FIDELITY = {"real", "synthetic", "mixed", "staged"}
_ALLOWED_WORK_ITEM_STATUS = {"open", "in_progress", "done", "blocked", "deferred"}

DEFAULT_SCHEMA_VERSION = 1
CURRENT_PROFILES_VERSION = 1
MAX_DESCRIPTION_LENGTH = 4000


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _coerce_stage(stage: str | None) -> str | None:
    if stage is None:
        return None
    stage = stage.strip().lower().replace("_", "-")
    if stage == "released":
        return "released"
    if stage == "productcomplete":
        return "product-complete"
    if stage == "benchmarkcomplete":
        return "benchmark-complete"
    if stage == "implementationcomplete":
        return "implementation-complete"
    return stage if stage in RELEASE_STATE_INDEX else None


def _iso_timestamp(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    trimmed = value.strip()
    if not trimmed:
        return False
    candidate = trimmed.replace("Z", "+00:00")
    try:
        datetime.fromisoformat(candidate)
        return True
    except ValueError:
        return False


def _sorted_join(values: set[str] | tuple[str, ...]) -> str:
    return ", ".join(f"'{value}'" for value in sorted(values))


def _add_error(errors: list[str], message: str) -> None:
    errors.append(message)


def _truncate_error(msg: str, max_len: int = 200) -> str:
    return msg if len(msg) <= max_len else msg[:max_len] + "..."


# ---------------------------------------------------------------------------
# Extraction / Parsing
# ---------------------------------------------------------------------------

def extract_exit_evidence(description: str) -> str | None:
    """Extract a JSON block from a milestone description."""
    for match in _BLOCK_FENCE_PATTERN.finditer(description):
        block = match.group(1).strip()
        try:
            parsed = json.loads(block)
        except json.JSONDecodeError:
            continue
        if not isinstance(parsed, dict):
            continue
        if "release_stage" in parsed or "schema_version" in parsed:
            return block
    stripped = description.strip()
    if stripped.startswith("{") and stripped.endswith("}"):
        try:
            json.loads(stripped)
            return stripped
        except json.JSONDecodeError:
            return None
    return None


def parse_exit_evidence(description: str) -> tuple[dict[str, Any] | None, list[str]]:
    """Return (payload, errors)."""
    block: str | None = None
    parse_errors: list[str] = []
    for match in _BLOCK_FENCE_PATTERN.finditer(description):
        candidate_block = match.group(1).strip()
        try:
            parsed = json.loads(candidate_block)
        except json.JSONDecodeError as error:
            parse_errors.append(
                f"Exit evidence block is not valid JSON: {_truncate_error(str(error))}"
            )
            continue
        if not isinstance(parsed, dict):
            continue
        if "release_stage" in parsed or "schema_version" in parsed:
            block = candidate_block
            break
    if block is None and parse_errors:
        return (None, parse_errors)
    if block is None:
        return (
            None,
            [
                "No release-exit evidence block found."
                " Add a ```release-exit-evidence``` JSON block to the milestone description."
            ],
        )
    try:
        payload = json.loads(block)
    except json.JSONDecodeError as error:
        return (None, [f"Exit evidence block is not valid JSON: {_truncate_error(str(error))}"])
    if not isinstance(payload, dict):
        return (None, ["Exit evidence block must be a JSON object."])
    return payload, []


# ---------------------------------------------------------------------------
# Generation
# ---------------------------------------------------------------------------

def generate_exit_evidence(
    *,
    release_stage: str,
    schema_version: int = DEFAULT_SCHEMA_VERSION,
    benchmark_date: str | None = None,
    data_fidelity: str | None = None,
    fingerprints: dict[str, str] | None = None,
    results: dict[str, dict[str, Any]] | None = None,
    degradations: list[dict[str, Any]] | None = None,
    post_release_work: list[dict[str, Any]] | None = None,
    environment: dict[str, str] | None = None,
    artifacts: list[dict[str, Any]] | None = None,
    profiles: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Generate a complete exit-evidence payload. release_stage and schema_version are always
    included; all other fields are added only when their value is not None."""
    payload: dict[str, Any] = {
        "schema_version": schema_version,
        "release_stage": release_stage,
    }
    has_any_field = any(
        v is not None
        for v in (benchmark_date, data_fidelity, fingerprints, results,
                  degradations, environment, artifacts)
    )
    if not has_any_field:
        if post_release_work is not None:
            payload["post_release_work"] = post_release_work
        if profiles is not None:
            payload["profiles"] = profiles
        return payload
    benchmark: dict[str, Any] = {}
    if benchmark_date is not None:
        benchmark["benchmark_date"] = benchmark_date
    if data_fidelity is not None:
        benchmark["data_fidelity"] = data_fidelity
    if fingerprints is not None:
        benchmark["fingerprints"] = fingerprints
    if environment is not None:
        benchmark["environment"] = environment
    if results is not None:
        benchmark["results"] = results
    if degradations is not None:
        benchmark["degradations"] = degradations
    if artifacts is not None:
        benchmark["artifacts"] = artifacts
    if benchmark:
        payload["benchmark"] = benchmark
    if post_release_work is not None:
        payload["post_release_work"] = post_release_work
    if profiles is not None:
        payload["profiles"] = profiles
    return payload


def generate_exit_evidence_block(
    *,
    release_stage: str,
    schema_version: int = DEFAULT_SCHEMA_VERSION,
    benchmark_date: str | None = None,
    data_fidelity: str | None = None,
    fingerprints: dict[str, str] | None = None,
    results: dict[str, dict[str, Any]] | None = None,
    degradations: list[dict[str, Any]] | None = None,
    post_release_work: list[dict[str, Any]] | None = None,
    environment: dict[str, str] | None = None,
    artifacts: list[dict[str, Any]] | None = None,
    profiles: dict[str, Any] | None = None,
    fence: str = "release-exit-evidence",
) -> str:
    """Generate a fenced markdown exit-evidence block ready for milestone descriptions."""
    if fence not in {"release-exit-evidence", "json"}:
        fence = "release-exit-evidence"
    payload = generate_exit_evidence(
        release_stage=release_stage, schema_version=schema_version,
        benchmark_date=benchmark_date, data_fidelity=data_fidelity,
        fingerprints=fingerprints, results=results, degradations=degradations,
        post_release_work=post_release_work, environment=environment,
        artifacts=artifacts, profiles=profiles,
    )
    return f"```{fence}\n{json.dumps(payload, indent=2, sort_keys=True)}\n```\n"


# ---------------------------------------------------------------------------
# Validation helpers
# ---------------------------------------------------------------------------

def _validate_map(*, payload: Mapping[str, Any], key: str,
                  required_keys: tuple[str, ...], errors: list[str]) -> None:
    value = payload.get(key)
    if not isinstance(value, Mapping):
        _add_error(errors, f"`{key}` must be an object.")
        return
    for required_key in required_keys:
        candidate = value.get(required_key)
        if not isinstance(candidate, str) or not candidate.strip():
            _add_error(errors, f"`{key}.{required_key}` must be a non-empty string.")


def _validate_results(*, results: Mapping[str, Any], errors: list[str]) -> None:
    for result_key in ("quality", "resource", "security"):
        result = results.get(result_key)
        if not isinstance(result, Mapping):
            _add_error(errors, f"`benchmark.results.{result_key}` must be an object.")
            continue
        status = result.get("status")
        if status not in _ALLOWED_RESULT_STATUS:
            _add_error(errors,
                       f"`benchmark.results.{result_key}.status` must be one of"
                       f" {_sorted_join(_ALLOWED_RESULT_STATUS)}. Got: {status!r}")


def _validate_degradations(*, degradations: Any, errors: list[str]) -> None:
    if not isinstance(degradations, list):
        _add_error(errors, "`benchmark.degradations` must be a list.")
        return
    for idx, item in enumerate(degradations):
        if isinstance(item, Mapping):
            continue
        if not isinstance(item, str) or not item.strip():
            _add_error(errors,
                       f"`benchmark.degradations[{idx}]` must be a string or object."
                       " A plain object is recommended for traceability.")


def _validate_post_release_work(*, required: bool, require_maintenance_group: bool,
                                work_items: Any, errors: list[str],
                                maintain_group: str = "maintenance/release") -> None:
    if not required:
        return
    if not isinstance(work_items, list) or not work_items:
        _add_error(errors,
                   "`post_release_work` is required while benchmark data is synthetic/pending,"
                   " or for stages that intentionally defer follow-up work"
                   " (`benchmark-complete` and `released`).")
        return
    if not require_maintenance_group:
        return
    for item in work_items:
        if not isinstance(item, Mapping):
            continue
        group = str(item.get("group") or "").strip()
        if group == maintain_group:
            return
    _add_error(errors,
               f"`post_release_work` is missing a `{maintain_group}`"
               " group entry for known pending follow-up work.")


def _validate_environment(*, environment: Any, errors: list[str]) -> None:
    if environment is None:
        return
    _validate_map(payload={"environment": environment}, key="environment",
                  required_keys=_REQUIRED_ENVIRONMENT_KEYS, errors=errors)


def _validate_artifacts(*, artifacts: Any, errors: list[str]) -> None:
    if artifacts is None:
        return
    if not isinstance(artifacts, list):
        _add_error(errors, "`benchmark.artifacts` must be a list.")
        return
    for idx, item in enumerate(artifacts):
        if not isinstance(item, Mapping):
            _add_error(errors, f"`benchmark.artifacts[{idx}]` must be an object.")
            continue
        source = item.get("source", "")
        if source not in _ALLOWED_ARTIFACT_SOURCES:
            _add_error(errors,
                       f"`benchmark.artifacts[{idx}].source` must be one of"
                       f" {_sorted_join(_ALLOWED_ARTIFACT_SOURCES)}. Got: {source!r}")
        url = item.get("url", "")
        if not isinstance(url, str) or not url.strip():
            _add_error(errors, f"`benchmark.artifacts[{idx}].url` must be a non-empty URL string.")
        label = item.get("label", "")
        if not isinstance(label, str) or not label.strip():
            _add_error(errors, f"`benchmark.artifacts[{idx}].label` must be a non-empty label string.")


def _validate_profiles(*, profiles: Any, errors: list[str]) -> None:
    if profiles is None:
        return
    if not isinstance(profiles, Mapping):
        _add_error(errors, "`profiles` must be an object.")
        return
    profiles_version = profiles.get("version", None)
    if profiles_version is not None and profiles_version != CURRENT_PROFILES_VERSION:
        _add_error(errors, f"`profiles.version` must be {CURRENT_PROFILES_VERSION}. Got: {profiles_version!r}")
    entries = profiles.get("entries")
    if entries is not None:
        if not isinstance(entries, list):
            _add_error(errors, "`profiles.entries` must be a list.")
            return
        for idx, entry in enumerate(entries):
            if not isinstance(entry, Mapping):
                _add_error(errors, f"`profiles.entries[{idx}]` must be an object.")
                continue
            stage = entry.get("stage", "")
            if stage not in _ALLOWED_PROFILE_STAGES:
                _add_error(errors,
                           f"`profiles.entries[{idx}].stage` must be one of"
                           f" {_sorted_join(_ALLOWED_PROFILE_STAGES)}. Got: {stage!r}")
            name = entry.get("name", "")
            if not isinstance(name, str) or not name.strip():
                _add_error(errors, f"`profiles.entries[{idx}].name` must be a non-empty string.")


# ---------------------------------------------------------------------------
# Validation entry point
# ---------------------------------------------------------------------------

def validate_exit_evidence(
    payload: Mapping[str, Any],
    *,
    required_stage: str = "product-complete",
    require_maintenance_group: bool = False,
) -> tuple[str | None, list[str]]:
    """Return (normalized_stage, errors)."""
    errors: list[str] = []
    if not isinstance(payload, Mapping):
        return None, ["Exit evidence payload must be an object."]
    if "schema_version" not in payload:
        _add_error(errors, "`schema_version` is required.")
    else:
        sv = payload.get("schema_version")
        if sv not in {1, "1"}:
            _add_error(errors, "Only schema_version=1 is supported.")
    stage = _coerce_stage(str(payload.get("release_stage")))
    if not stage:
        _add_error(errors, "`release_stage` must be one of"
                   f" {_sorted_join(set(RELEASE_STATES))}.")
        return None, errors
    if _coerce_stage(required_stage) not in RELEASE_STATE_INDEX:
        _add_error(errors, "`required_stage` must be one of"
                   f" {_sorted_join(set(RELEASE_STATES))}.")
        return stage, errors
    if RELEASE_STATE_INDEX[stage] < RELEASE_STATE_INDEX[_coerce_stage(required_stage)]:
        _add_error(errors, f"Release preflight requires at least '{required_stage}' stage.")
        return stage, errors
    _validate_profiles(profiles=payload.get("profiles"), errors=errors)
    if stage == "implementation-complete":
        return stage, errors
    benchmark = payload.get("benchmark")
    if not isinstance(benchmark, Mapping):
        _add_error(errors, "`benchmark` section is required for this stage.")
        return stage, errors
    if _coerce_stage(stage) in {"benchmark-complete", "product-complete", "released"}:
        bd = benchmark.get("benchmark_date")
        if not _iso_timestamp(bd):
            _add_error(errors, "`benchmark.benchmark_date` must be an ISO date or datetime string.")
        df = benchmark.get("data_fidelity")
        if df not in _ALLOWED_DATA_FIDELITY:
            _add_error(errors, "`benchmark.data_fidelity` must be one of"
                       f" {_sorted_join(_ALLOWED_DATA_FIDELITY)}.")
        _validate_map(payload=benchmark, key="fingerprints",
                      required_keys=_REQUIRED_FINGERPRINT_KEYS, errors=errors)
        results = benchmark.get("results")
        if not isinstance(results, Mapping):
            _add_error(errors, "`benchmark.results` must be an object.")
        else:
            _validate_results(results=results, errors=errors)
            if stage in {"product-complete", "released"}:
                for rk in ("quality", "resource", "security"):
                    r = results.get(rk)
                    if isinstance(r, Mapping) and r.get("status") != "pass":
                        _add_error(errors,
                                   f"`benchmark.results.{rk}.status` must be 'pass'"
                                   " for product-complete/released release stages.")
        _validate_degradations(degradations=benchmark.get("degradations"), errors=errors)
        _validate_environment(environment=benchmark.get("environment"), errors=errors)
        _validate_artifacts(artifacts=benchmark.get("artifacts"), errors=errors)
        pending = df == "synthetic" or df == "staged"
        req_prw = stage == "benchmark-complete" or stage == "released" or (
            stage == "product-complete" and pending)
        _validate_post_release_work(required=req_prw,
                                    require_maintenance_group=require_maintenance_group and req_prw,
                                    work_items=payload.get("post_release_work"),
                                    errors=errors, maintain_group="maintenance/release")
        if stage in {"product-complete", "released"} and df != "real":
            _add_error(errors,
                       "`benchmark.data_fidelity` must be 'real' for product-complete"
                       " and released stages. Synthetic or staged measurements are insufficient.")
    return stage, errors


# ---------------------------------------------------------------------------
# Reconciliation
# ---------------------------------------------------------------------------

RELEASE_RECONCILIATION_RULES: dict[str, dict[str, dict[str, Any]]] = {
    "quality": {"p50": {"type": "float", "required": True},
                "p95": {"type": "float", "required": False}},
    "resource": {"p95_latency_ms": {"type": "float", "required": True},
                  "memory_mb": {"type": "float", "required": False}},
    "security": {"violations": {"type": "int", "required": True}},
}


def reconcile_exit_evidence(
    *,
    evidence_payload: dict[str, Any],
    actual_results: dict[str, dict[str, Any]] | None = None,
    actual_environments: dict[str, str] | None = None,
) -> list[dict[str, str]]:
    """Compare exit evidence against actual (reference) measurement data.
    Returns a list of reconciliation issues, each with keys kind, field, detail."""
    issues: list[dict[str, str]] = []
    benchmark = evidence_payload.get("benchmark")
    evidence_results = benchmark.get("results") if isinstance(benchmark, dict) else None
    if actual_results is not None:
        for rk, rules in RELEASE_RECONCILIATION_RULES.items():
            ev_res = evidence_results.get(rk) if isinstance(evidence_results, dict) else None
            ac_res = actual_results.get(rk) if isinstance(actual_results, dict) else None
            for fn, fr in rules.items():
                if not fr.get("required", False):
                    continue
                ev_val = None
                if ev_res is not None and isinstance(ev_res, dict):
                    ev_val = ev_res.get(fn)
                    if ev_val is None:
                        issues.append({"kind": "missing", "field": f"benchmark.results.{rk}.{fn}",
                                       "detail": f"Evidence is missing required `{rk}.{fn}`."})
                if ac_res is not None and isinstance(ac_res, dict):
                    ac_val = ac_res.get(fn)
                    if ac_val is None:
                        issues.append({"kind": "missing", "field": f"actual_results.{rk}.{fn}",
                                       "detail": f"Reference data is missing `{rk}.{fn}`."})
                    if ev_val is not None and ac_val is not None and ev_val != ac_val:
                        issues.append({"kind": "mismatch", "field": f"benchmark.results.{rk}.{fn}",
                                       "detail": f"Evidence value {ev_val!r} does not match actual {ac_val!r}."})
    if actual_environments is not None:
        ev_env = benchmark.get("environment") if isinstance(benchmark, dict) else None
        for ek in _REQUIRED_ENVIRONMENT_KEYS:
            ac_val = actual_environments.get(ek)
            ev_val = None
            if ev_env is not None and isinstance(ev_env, dict):
                ev_val = ev_env.get(ek)
            if ev_val is None:
                issues.append({"kind": "env_missing", "field": f"benchmark.environment.{ek}",
                               "detail": f"Evidence is missing environment key `{ek}` (actual: {ac_val!r})."})
            elif ac_val is not None and ev_val != ac_val:
                issues.append({"kind": "env_drift", "field": f"benchmark.environment.{ek}",
                               "detail": f"Environment `{ek}` drifted: evidence {ev_val!r} vs actual {ac_val!r}."})
    return issues


# ---------------------------------------------------------------------------
# Post-release validation
# ---------------------------------------------------------------------------

def validate_post_release_tracking(
    *,
    work_items: list[dict[str, Any]] | None,
    follow_up_issues: dict[str, str] | None = None,
) -> tuple[bool, list[str]]:
    """Validate post-release work item statuses and completeness.
    Returns (is_valid, messages)."""
    messages: list[str] = []
    if not work_items:
        return True, messages
    tracked_groups: set[str] = set()
    for idx, item in enumerate(work_items):
        if not isinstance(item, dict):
            messages.append(f"Work item {idx} must be a dict.")
            continue
        group = str(item.get("group") or "").strip()
        if not group:
            messages.append(f"Work item {idx} is missing a `group`.")
            continue
        tracked_groups.add(group)
        status = str(item.get("status") or "").strip()
        if not status:
            messages.append(f"Work item {idx} ('{group}') is missing a `status`.")
        elif status not in _ALLOWED_WORK_ITEM_STATUS:
            messages.append(f"Work item {idx} ('{group}'): status {status!r}"
                            f" not in {_sorted_join(_ALLOWED_WORK_ITEM_STATUS)}.")
        if status == "done" and not item.get("issue"):
            messages.append(f"Work item {idx} ('{group}') is marked 'done' but has no `issue` URL.")
    if follow_up_issues is not None:
        for group in sorted(tracked_groups):
            if not follow_up_issues.get(group):
                messages.append(f"Follow-up issue missing for group {group!r}.")
    return len(messages) == 0, messages


# ---------------------------------------------------------------------------
# File-based validation entry point
# ---------------------------------------------------------------------------

def validate_from_file(
    *,
    path: Path,
    required_stage: str = "product-complete",
    require_maintenance_group: bool = False,
    milestone_title: str = "",
) -> int:
    text = path.read_text(encoding="utf-8")
    payload, parse_errors = parse_exit_evidence(text)
    if parse_errors:
        for issue in parse_errors:
            print(f"{issue}")
        return 1
    stage, errors = validate_exit_evidence(
        payload, required_stage=required_stage,
        require_maintenance_group=require_maintenance_group,
    )
    if errors:
        title = f" for milestone '{milestone_title}'" if milestone_title else ""
        print(f"Release evidence gate failed{title}:")
        for issue in errors:
            print(f"  - {issue}")
        return 1
    if not stage:
        print("Could not determine release stage from exit evidence.")
        return 1
    print(stage)
    return 0


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    vp = sub.add_parser("validate", help="Validate a milestone description file.")
    vp.add_argument("--description-file", required=True, type=Path, help="Path to the milestone description text file.")
    vp.add_argument("--required-stage", default="product-complete", choices=RELEASE_STATES, help="Minimum accepted release stage.")
    vp.add_argument("--require-maintenance-grouping", action="store_true", help="Require synthetic/staged benchmark follow-up work in maintenance/release grouping when supported.")
    vp.add_argument("--milestone-title", default="", help="Optional milestone title for diagnostics.")

    gp = sub.add_parser("generate", help="Generate an exit-evidence block (print to stdout).")
    gp.add_argument("--release-stage", required=True, choices=RELEASE_STATES, help="Release milestone stage.")
    gp.add_argument("--schema-version", type=int, default=DEFAULT_SCHEMA_VERSION, help=f"Schema version (default {DEFAULT_SCHEMA_VERSION}).")
    gp.add_argument("--benchmark-date", default="", help="ISO date or datetime of the benchmark.")
    gp.add_argument("--data-fidelity", default="", choices=sorted(_ALLOWED_DATA_FIDELITY), help="Benchmark data fidelity level.")
    gp.add_argument("--corpus-snapshot", default="", help="Fingerprint: corpus snapshot identifier.")
    gp.add_argument("--index-generation", default="", help="Fingerprint: index generation identifier.")
    gp.add_argument("--model-fingerprint", default="", help="Fingerprint: model/provider fingerprint.")
    gp.add_argument("--environment-os", default="", help="Environment: operating system.")
    gp.add_argument("--environment-rust-toolchain", default="", help="Environment: Rust toolchain channel.")
    gp.add_argument("--environment-cpu-arch", default="", help="Environment: CPU architecture.")
    gp.add_argument("--fence", default="release-exit-evidence", choices={"release-exit-evidence", "json"}, help="Fence marker type.")

    rp = sub.add_parser("reconcile", help="Reconcile exit evidence against actual metrics.")
    rp.add_argument("--evidence-file", required=True, type=Path, help="Path to the evidence description file.")
    rp.add_argument("--actual-results-file", type=Path, default=None, help="Path to JSON file with actual benchmark results.")
    rp.add_argument("--actual-environment-file", type=Path, default=None, help="Path to JSON file with actual environment data.")

    tp = sub.add_parser("validate-tracking", help="Validate post-release tracking completeness.")
    tp.add_argument("--work-items-file", required=True, type=Path, help="Path to JSON file containing post_release_work items array.")
    tp.add_argument("--follow-up-issues-file", type=Path, default=None, help="Path to JSON file mapping groups to issue URLs.")

    return parser


def _cmd_validate(args: argparse.Namespace) -> int:
    return validate_from_file(path=args.description_file, required_stage=args.required_stage,
                              require_maintenance_group=args.require_maintenance_grouping,
                              milestone_title=args.milestone_title)


def _cmd_generate(args: argparse.Namespace) -> int:
    fingerprints: dict[str, str] = {}
    if args.corpus_snapshot:
        fingerprints["corpus_snapshot"] = args.corpus_snapshot
    if args.index_generation:
        fingerprints["index_generation"] = args.index_generation
    if args.model_fingerprint:
        fingerprints["model_fingerprint"] = args.model_fingerprint
    environment: dict[str, str] = {}
    if args.environment_os:
        environment["os"] = args.environment_os
    if args.environment_rust_toolchain:
        environment["rust_toolchain"] = args.environment_rust_toolchain
    if args.environment_cpu_arch:
        environment["cpu_arch"] = args.environment_cpu_arch
    block = generate_exit_evidence_block(
        release_stage=args.release_stage, schema_version=args.schema_version,
        benchmark_date=args.benchmark_date or None, data_fidelity=args.data_fidelity or None,
        fingerprints=fingerprints or None, results=None, degradations=None,
        post_release_work=None, environment=environment or None, artifacts=None,
        profiles=None, fence=args.fence)
    sys.stdout.write(block)
    return 0


def _cmd_reconcile(args: argparse.Namespace) -> int:
    text = args.evidence_file.read_text(encoding="utf-8")
    payload, parse_errors = parse_exit_evidence(text)
    if parse_errors:
        for issue in parse_errors:
            print(f"{issue}")
        return 1
    if payload is None:
        print("Could not parse exit evidence.")
        return 1
    actual_results = None
    if args.actual_results_file is not None:
        raw = json.loads(args.actual_results_file.read_text(encoding="utf-8"))
        actual_results = raw if isinstance(raw, dict) else None
    actual_environments = None
    if args.actual_environment_file is not None:
        raw = json.loads(args.actual_environment_file.read_text(encoding="utf-8"))
        actual_environments = raw if isinstance(raw, dict) else None
    issues = reconcile_exit_evidence(evidence_payload=payload, actual_results=actual_results,
                                      actual_environments=actual_environments)
    if not issues:
        print("Reconciliation passed — no issues found.")
        return 0
    print(f"Reconciliation found {len(issues)} issue(s):")
    for issue in issues:
        print(f"  - [{issue['kind']}] {issue['field']}: {issue['detail']}")
    return 1


def _cmd_validate_tracking(args: argparse.Namespace) -> int:
    raw = json.loads(args.work_items_file.read_text(encoding="utf-8"))
    work_items: list[dict[str, Any]] = raw if isinstance(raw, list) else []
    follow_up_issues = None
    if args.follow_up_issues_file is not None:
        raw_fu = json.loads(args.follow_up_issues_file.read_text(encoding="utf-8"))
        follow_up_issues = raw_fu if isinstance(raw_fu, dict) else None
    is_valid, messages = validate_post_release_tracking(work_items=work_items, follow_up_issues=follow_up_issues)
    if is_valid:
        print("Post-release tracking validation passed.")
        return 0
    for msg in messages:
        print(f"  - {msg}")
    return 1


COMMAND_MAP: dict[str, Any] = {
    "validate": _cmd_validate, "generate": _cmd_generate,
    "reconcile": _cmd_reconcile, "validate-tracking": _cmd_validate_tracking,
}


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()
    handler = COMMAND_MAP.get(args.command)
    if handler is not None:
        return handler(args)
    parser.print_help()
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
