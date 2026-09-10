#!/usr/bin/env python3
"""Validate Maestria's checked-in benchmark evidence ledger and run reports."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
ALLOWED_FIDELITY = {"real", "synthetic", "mixed", "staged"}
ALLOWED_STATUS = {"pass", "warning", "fail", "pending", "n/a"}
REQUIRED_MILESTONES = (
    "v0.4 — Deterministic Search Baseline",
    "v0.5 — Evaluated Hybrid Retrieval",
    "v0.7 — Repository Intelligence",
    "v0.8 — Visual Document Retrieval",
    "v0.9 — Doc & Marker Search",
    "v1.0 — Python Repository Intelligence",
    "v1.1 — Web Repository Intelligence",
    "v1.2 — Learned-Sparse Four-Profile Evaluation",
    "v1.3 — Dense-Lane Promotion (re-justified judgment set v2)",
)

def measurement_status_is_valid(status: Any) -> bool:
    if status == "Measured":
        return True
    if not isinstance(status, dict) or len(status) != 1:
        return False
    if status.get("Measured") is None and "Measured" in status:
        return True
    for variant in ("Unavailable", "NotApplicable"):
        payload = status.get(variant)
        if isinstance(payload, dict) and str(payload.get("reason", "")).strip():
            return True
    return False


def enum_variant(value: Any) -> str | None:
    if isinstance(value, str):
        return value
    if isinstance(value, dict) and len(value) == 1:
        variant = next(iter(value))
        return variant if isinstance(variant, str) else None
    return None
REQUIRED_RESULT_KEYS = ("quality", "resource", "security")
REQUIRED_ENVIRONMENT_KEYS = ("os", "rust_toolchain", "cpu_arch")


def errors_for_manifest(path: Path) -> list[str]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"{path}: cannot parse JSON: {error}"]
    if not isinstance(payload, dict):
        return ["manifest root must be an object"]

    errors: list[str] = []
    if payload.get("schema_version") != 1:
        errors.append("schema_version must be 1")
    if not isinstance(payload.get("measurement_policy"), dict):
        errors.append("measurement_policy must be an object")

    entries = payload.get("benchmarks")
    if not isinstance(entries, list):
        return errors + ["benchmarks must be a list"]

    for index, entry in enumerate(entries):
        prefix = f"benchmarks[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{prefix} must be an object")
            continue
        if not str(entry.get("benchmark", "")).strip():
            errors.append(f"{prefix}.benchmark must be a non-empty name")
        fidelity = entry.get("data_fidelity")
        if fidelity not in ALLOWED_FIDELITY:
            errors.append(f"{prefix}.data_fidelity is invalid: {fidelity!r}")

        corpus = entry.get("corpus")
        if not isinstance(corpus, dict):
            errors.append(f"{prefix}.corpus must be an object")
        else:
            for key in ("id", "snapshot", "judgment_set"):
                if not isinstance(corpus.get(key), str) or not corpus[key].strip():
                    errors.append(f"{prefix}.corpus.{key} must be non-empty")
            source_paths = corpus.get("source_paths")
            if not isinstance(source_paths, list) or not source_paths:
                errors.append(f"{prefix}.corpus.source_paths must be non-empty")
            else:
                for source in source_paths:
                    source_path = ROOT / str(source)
                    if not source_path.is_file():
                        errors.append(f"{prefix}.corpus source is missing: {source}")
                expected_hash = corpus.get("source_hash")
                if not isinstance(expected_hash, str) or len(expected_hash) != 64:
                    errors.append(f"{prefix}.corpus.source_hash must be a SHA-256 digest")
                elif source_paths:
                    digest = hashlib.sha256()
                    for source in source_paths:
                        source_path = ROOT / str(source)
                        if source_path.is_file():
                            digest.update(str(source).encode())
                            digest.update(b"\0")
                            digest.update(source_path.read_bytes())
                    if digest.hexdigest() != expected_hash:
                        errors.append(f"{prefix}.corpus.source_hash does not match source files")

        for container_name, required_keys in (
            ("fingerprints", ("corpus_snapshot", "index_generation", "model_fingerprint")),
            ("environment", REQUIRED_ENVIRONMENT_KEYS),
        ):
            container = entry.get(container_name)
            if not isinstance(container, dict):
                errors.append(f"{prefix}.{container_name} must be an object")
                continue
            for key in required_keys:
                value = container.get(key)
                if not isinstance(value, str) or not value.strip() or "<" in value:
                    errors.append(f"{prefix}.{container_name}.{key} must be concrete")

        results = entry.get("results")
        if not isinstance(results, dict):
            errors.append(f"{prefix}.results must be an object")
        else:
            for result_key in REQUIRED_RESULT_KEYS:
                result = results.get(result_key)
                if not isinstance(result, dict):
                    errors.append(f"{prefix}.results.{result_key} must be an object")
                    continue
                status = result.get("status")
                if status not in ALLOWED_STATUS:
                    errors.append(f"{prefix}.results.{result_key}.status is invalid")

        degradations = entry.get("degradations")
        if not isinstance(degradations, list):
            errors.append(f"{prefix}.degradations must be a list")
        elif any(not isinstance(item, dict) or not str(item.get("description", "")).strip()
                 for item in degradations):
            errors.append(f"{prefix}.degradations must contain descriptions")

        reports = entry.get("reports")
        if not isinstance(reports, list) or not reports:
            errors.append(f"{prefix}.reports must be a non-empty list")
        else:
            for report_index, report in enumerate(reports):
                if not isinstance(report, dict) or not report.get("kind") or not report.get("path"):
                    errors.append(f"{prefix}.reports[{report_index}] needs kind and path")

    return errors


def errors_for_report(
    path: Path, kind: str, manifest_entry: dict[str, Any] | None = None
) -> list[str]:
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        return [f"{path}: cannot parse report: {error}"]
    if not isinstance(report, dict):
        return [f"{path}: report root must be an object"]
    errors: list[str] = []
    if kind in {"golden", "hybrid"}:
        corpus = report.get("corpus")
        observations = report.get("observations")
        if not isinstance(corpus, dict) or not isinstance(observations, list) or not observations:
            return [f"{path}: golden reports need corpus and non-empty observations"]
        if kind == "hybrid" and not any(
            observation.get("profile") == "v0.5" for observation in observations
            if isinstance(observation, dict)
        ):
            errors.append(f"{path}: hybrid report needs v0.5 observations")
        for index, observation in enumerate(observations):
            if not isinstance(observation, dict):
                errors.append(f"{path}: observations[{index}] must be an object")
                continue
            for key in ("profile", "outcome", "resources", "security"):
                if key not in observation:
                    errors.append(f"{path}: observations[{index}] missing {key}")
        return errors

    required_identity = ("evaluation_date",) if kind == "late-interaction-stage-a-promotion" else (
        "measurement_kind",
        "evaluation_date",
    )
    for key in required_identity:
        if not str(report.get(key, "")).strip():
            errors.append(f"{path}: missing {key}")
    if kind == "repository":
        for key in ("corpus_id", "repository_revision", "index_generation", "model_fingerprint"):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        if manifest_entry is not None:
            corpus = manifest_entry.get("corpus", {})
            fingerprints = manifest_entry.get("fingerprints", {})
            for key, expected in (
                ("corpus_id", corpus.get("id")),
                ("index_generation", fingerprints.get("index_generation")),
                ("model_fingerprint", fingerprints.get("model_fingerprint")),
            ):
                if report.get(key) != expected:
                    errors.append(f"{path}: {key} is not bound to its manifest")
        observations = report.get("observations")
        if not isinstance(observations, list) or not observations:
            errors.append(f"{path}: observations must be non-empty")
        else:
            for index, observation in enumerate(observations):
                prefix = f"{path}: observations[{index}]"
                if not isinstance(observation, dict):
                    errors.append(f"{prefix} must be an object")
                    continue
                for key in ("case_id", "route", "latency_ms", "outcome_correct"):
                    if key not in observation:
                        errors.append(f"{prefix} missing {key}")
                if observation.get("route") not in {"PhaseC", "CodeSpecialized"}:
                    errors.append(f"{prefix}.route is invalid")
                for key in (
                    "latency_ms",
                    "exact_span_hits",
                    "evidence_chain_length",
                    "memory_bytes",
                    "disk_bytes",
                    "energy_milliwatt_seconds",
                ):
                    if not isinstance(observation.get(key), int) or observation[key] < 0:
                        errors.append(f"{prefix}.{key} must be a non-negative integer")
                if not isinstance(observation.get("outcome_correct"), bool):
                    errors.append(f"{prefix}.outcome_correct must be boolean")
                status = observation.get("measurement_status")
                if not (
                    status == "Measured"
                    or isinstance(status, dict)
                    and isinstance(status.get("Unavailable"), dict)
                    and str(status["Unavailable"].get("reason", "")).strip()
                ):
                    errors.append(f"{prefix}.measurement_status is invalid")
    elif kind == "build-latency":
        for key in ("corpus_id", "repository_revision", "index_generation", "model_fingerprint"):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        if manifest_entry is not None:
            corpus = manifest_entry.get("corpus", {})
            fingerprints = manifest_entry.get("fingerprints", {})
            for key, expected in (
                ("corpus_id", corpus.get("id")),
                ("index_generation", fingerprints.get("index_generation")),
                ("model_fingerprint", fingerprints.get("model_fingerprint")),
            ):
                if report.get(key) != expected:
                    errors.append(f"{path}: {key} is not bound to its manifest")
        sizes = report.get("sizes")
        if not isinstance(sizes, list) or not sizes:
            errors.append(f"{path}: sizes must be non-empty")
        else:
            for size_index, size in enumerate(sizes):
                prefix = f"{path}: sizes[{size_index}]"
                if not isinstance(size, dict):
                    errors.append(f"{prefix} must be an object")
                    continue
                for key in ("files", "symbols", "runs", "p50_ms", "p95_ms"):
                    if not isinstance(size.get(key), int) or size[key] < 0:
                        errors.append(f"{prefix}.{key} must be a non-negative integer")
                if not isinstance(size.get("measurements_ms"), list) or not size[
                    "measurements_ms"
                ]:
                    errors.append(f"{prefix}.measurements_ms must be non-empty")
                elif any(
                    not isinstance(value, int) or value < 0
                    for value in size["measurements_ms"]
                ):
                    errors.append(f"{prefix}.measurements_ms must be non-negative integers")
    elif kind == "visual":
        if report.get("provider_status") != "unavailable":
            errors.append(f"{path}: visual report must state provider_status=unavailable")
        if not isinstance(report.get("observations"), list) or not report["observations"]:
            errors.append(f"{path}: observations must be non-empty")
    elif kind == "visual-provider":
        for key in ("measurement_kind", "evaluation_date", "corpus_id", "corpus_revision"):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        observations = report.get("observations")
        if not isinstance(observations, list) or not observations:
            errors.append(f"{path}: observations must be non-empty")
    elif kind == "late-interaction-stage-a":
        for key in ("measurement_kind", "evaluation_date", "evaluation_id", "stage"):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        if report.get("stage") != "Reranker":
            errors.append(f"{path}: stage must be Reranker")
        fidelity = report.get("data_fidelity")
        if fidelity not in {"real", "mixed", "staged"}:
            errors.append(f"{path}: Stage A data_fidelity must be real, mixed, or staged")
        corpus = report.get("corpus")
        if not isinstance(corpus, dict):
            errors.append(f"{path}: corpus must be an object")
        else:
            for key in ("id", "revision", "judgment_set", "source_hash", "judgment_hash"):
                if not str(corpus.get(key, "")).strip():
                    errors.append(f"{path}: corpus.{key} must be concrete")
        fingerprints = report.get("fingerprints")
        if not isinstance(fingerprints, dict):
            errors.append(f"{path}: fingerprints must be an object")
        else:
            for key in (
                "corpus_snapshot",
                "index_generation",
                "identity_digest",
                "profile_digest",
                "scorer_fingerprint",
                "source_hash",
                "judgment_hash",
            ):
                if not str(fingerprints.get(key, "")).strip() or "<" in str(
                    fingerprints.get(key, "")
                ):
                    errors.append(f"{path}: fingerprints.{key} must be concrete")
        observations = report.get("observations")
        if not isinstance(observations, list) or not observations:
            errors.append(f"{path}: observations must be non-empty")
        else:
            allowed_routes = {
                "LexicalExact",
                "EligibleHybrid",
                "EligibleBoundedBaseline",
                "LateInteractionReranker",
            }
            observed_pairs: set[tuple[str, str]] = set()
            for index, observation in enumerate(observations):
                prefix = f"{path}: observations[{index}]"
                if not isinstance(observation, dict):
                    errors.append(f"{prefix} must be an object")
                    continue
                for key in ("case_id", "query_class", "route", "quality", "resources", "safety"):
                    if key not in observation:
                        errors.append(f"{prefix} missing {key}")
                route = observation.get("route")
                query_class = observation.get("query_class")
                if route not in allowed_routes:
                    errors.append(f"{prefix}.route is invalid")
                elif isinstance(query_class, str):
                    observed_pairs.add((query_class, route))
                for container in ("quality", "resources", "safety"):
                    if not isinstance(observation.get(container), dict) or not observation[container]:
                        errors.append(f"{prefix}.{container} must be a non-empty object")
                status = observation.get("measurement_status")
                if not measurement_status_is_valid(status):
                    errors.append(f"{prefix}.measurement_status is invalid")
            if fidelity != "staged":
                decision_map = report.get("decisions")
                expected_classes = (
                    {class_name for class_name in decision_map if isinstance(class_name, str)}
                    if isinstance(decision_map, dict)
                    else set()
                )
                required_pairs = {
                    (class_name, route)
                    for class_name in expected_classes
                    for route in allowed_routes
                }
                missing_pairs = sorted(required_pairs - observed_pairs)
                if missing_pairs:
                    errors.append(
                        f"{path}: complete Stage A class/route coverage is missing "
                        f"{missing_pairs[:8]}"
                    )
        decisions = report.get("decisions")
        if not isinstance(decisions, dict) or not decisions:
            errors.append(f"{path}: decisions must be a non-empty map")
        else:
            allowed_decisions = {
                "RetainBaseline",
                "RetainLateInteractionReranker",
                "RejectExperiment",
            }
            for class_name, decision in decisions.items():
                if decision not in allowed_decisions:
                    errors.append(f"{path}: decisions[{class_name}] is invalid")
            protected = {"Exact", "ExactLiteral", "Path", "Filename", "Identifier", "Phrase", "Metadata", "Symbol"}
            if any(
                class_name in protected and decision == "RetainLateInteractionReranker"
                for class_name, decision in decisions.items()
            ):
                errors.append(f"{path}: protected classes cannot use late interaction")
            if fidelity == "staged" and any(
                decision == "RetainLateInteractionReranker"
                for decision in decisions.values()
            ):
                errors.append(f"{path}: staged Stage A evidence cannot retain late interaction")
    elif kind == "late-interaction-stage-a-promotion":
        for key in (
            "schema_version",
            "evaluation_id",
            "evaluation_date",
            "corpus_id",
            "corpus_revision",
            "stage_a_report_hash",
            "profile_identity",
            "generation_id",
            "corpus_snapshot",
            "rollback_generation_id",
            "promoted_classes",
            "authorized",
        ):
            if key not in report:
                errors.append(f"{path}: missing {key}")
        if report.get("schema_version") != 1:
            errors.append(f"{path}: promotion schema_version must be 1")
        report_hash = report.get("stage_a_report_hash")
        if (
            not isinstance(report_hash, str)
            or len(report_hash) != 71
            or not report_hash.startswith("sha256:")
        ):
            errors.append(f"{path}: stage_a_report_hash must be a SHA-256 digest")
        for key in (
            "evaluation_id",
            "evaluation_date",
            "corpus_id",
            "corpus_revision",
            "profile_identity",
            "generation_id",
            "corpus_snapshot",
            "rollback_generation_id",
        ):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: {key} must be concrete")
        promoted_classes = report.get("promoted_classes")
        if (
            not isinstance(promoted_classes, list)
            or not promoted_classes
            or any(not isinstance(value, str) or not value.strip() for value in promoted_classes)
        ):
            errors.append(f"{path}: promoted_classes must be a non-empty string list")
        if report.get("authorized") is not True:
            errors.append(f"{path}: promotion record must be authorized")
    elif kind == "late-interaction-stage-b":
        for key in (
            "measurement_kind",
            "evaluation_date",
            "evaluation_id",
            "stage",
            "stage_a_report_hash",
            "decision",
        ):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        if report.get("stage") != "CandidateIndex":
            errors.append(f"{path}: stage must be CandidateIndex")
        decision = enum_variant(report.get("decision"))
        allowed_decisions = {
            "NotAuthorized",
            "AuthorizedForEvaluation",
            "RetainStageA",
            "PromoteMultiVectorIndex",
            "RejectIndex",
        }
        if decision not in allowed_decisions:
            errors.append(f"{path}: decision is invalid")
        indexed_need = report.get("indexed_need")
        indexed_status: str | None = None
        indexed_case_ids: Any = None
        if not isinstance(indexed_need, dict):
            errors.append(f"{path}: indexed_need must be an object")
        elif "status" in indexed_need:
            indexed_status = indexed_need.get("status")
            indexed_case_ids = indexed_need.get("case_ids")
            if indexed_status not in {"MeasuredNeed", "NoMeasuredNeed", "Unavailable"}:
                errors.append(f"{path}: indexed_need.status is invalid")
            if not isinstance(indexed_case_ids, list):
                errors.append(f"{path}: indexed_need.case_ids must be a list")
            if indexed_status != "MeasuredNeed" and not str(
                indexed_need.get("reason", "")
            ).strip():
                errors.append(f"{path}: indexed_need.reason is required")
        else:
            indexed_status = enum_variant(indexed_need)
            payload = indexed_need.get(indexed_status) if indexed_status else None
            if indexed_status not in {"MeasuredNeed", "NoMeasuredNeed", "Unavailable"}:
                errors.append(f"{path}: indexed_need variant is invalid")
            elif not isinstance(payload, dict):
                errors.append(f"{path}: indexed_need.{indexed_status} must be an object")
            else:
                indexed_case_ids = payload.get("case_ids", [])
                if not isinstance(indexed_case_ids, list):
                    errors.append(f"{path}: indexed_need.case_ids must be a list")
                if indexed_status != "MeasuredNeed" and not str(
                    payload.get("reason", "")
                ).strip():
                    errors.append(f"{path}: indexed_need reason is required")
        if decision in {"AuthorizedForEvaluation", "PromoteMultiVectorIndex"}:
            if indexed_status != "MeasuredNeed":
                errors.append(f"{path}: index authorization requires measured need")
            if report.get("stage_a_quality_win") is not True:
                errors.append(f"{path}: index authorization requires a Stage A quality win")
    elif kind == "learned-sparse":
        for key in (
            "measurement_kind",
            "evaluation_date",
            "corpus_id",
            "corpus_revision",
            "index_generation",
            "model_fingerprint",
            "namespace",
        ):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        route_configuration = report.get("route_configuration")
        if not isinstance(route_configuration, dict) or route_configuration.get("route") != "SparseFused":
            errors.append(f"{path}: route_configuration must state the SparseFused route")
        if manifest_entry is not None:
            corpus = manifest_entry.get("corpus", {})
            fingerprints = manifest_entry.get("fingerprints", {})
            for key, expected in (
                ("corpus_id", corpus.get("id")),
                ("index_generation", fingerprints.get("index_generation")),
                ("model_fingerprint", fingerprints.get("model_fingerprint")),
            ):
                if report.get(key) != expected:
                    errors.append(f"{path}: {key} is not bound to its manifest")
        observations = report.get("observations")
        if not isinstance(observations, list) or not observations:
            errors.append(f"{path}: observations must be non-empty")
        else:
            for index, observation in enumerate(observations):
                prefix = f"{path}: observations[{index}]"
                if not isinstance(observation, dict):
                    errors.append(f"{prefix} must be an object")
                    continue
                for key in ("case_id", "route", "quality", "resources", "safety"):
                    if not isinstance(observation.get(key), (str, dict)) or not observation[key]:
                        errors.append(f"{prefix} missing or empty {key}")
                if observation.get("route") not in {
                    "Lexical",
                    "Hybrid",
                    "SparseOnly",
                    "SparseFused",
                }:
                    errors.append(f"{prefix}.route is invalid")
                for container in ("quality", "resources", "safety"):
                    value = observation.get(container)
                    if isinstance(value, dict) and not value:
                        errors.append(f"{prefix}.{container} must be non-empty")
                status = observation.get("measurement_status")
                if not (
                    status == "Measured"
                    or isinstance(status, dict)
                    and isinstance(status.get("Unavailable"), dict)
                    and str(status["Unavailable"].get("reason", "")).strip()
                ):
                    errors.append(f"{prefix}.measurement_status is invalid")
        decisions = report.get("decisions")
        if not isinstance(decisions, dict) or not decisions:
            errors.append(f"{path}: decisions must be a non-empty map")
        else:
            for class_name in (
                "ExactLiteral",
                "VocabularyExpansion",
                "DomainTerminology",
                "MultiTerm",
                "NoEvidence",
                "Security",
            ):
                decision = decisions.get(class_name)
                if decision not in {
                    "PromoteSparseFused",
                    "RetainHybrid",
                    "RetainLexical",
                    "RemainShadowed",
                    "Disable",
                }:
                    errors.append(f"{path}: decisions[{class_name}] is invalid")
            if any(
                decision == "PromoteSparseFused"
                for class_name, decision in decisions.items()
                if class_name in {"ExactLiteral", "NoEvidence", "Security"}
            ):
                errors.append(f"{path}: protected classes cannot be promoted")
    else:
        errors.append(f"{path}: unknown report kind {kind!r}")
    return errors


def report_path(report: dict[str, Any], report_root: Path | None) -> Path:
    path = Path(str(report["path"]))
    if report_root is not None and path.parts[:2] == ("target", "benchmark-reports"):
        return report_root / path.name
    return ROOT / path


def validate(manifest: Path, report_root: Path | None) -> int:
    errors = errors_for_manifest(manifest)
    if report_root is not None:
        try:
            payload = json.loads(manifest.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            payload = {}
        stage_a_hashes: set[str] = set()
        stage_b_bindings: list[tuple[Path, str]] = []
        for entry in payload.get("benchmarks", []):
            if not isinstance(entry, dict):
                continue
            for report in entry.get("reports", []):
                if not isinstance(report, dict) or not report.get("kind") or not report.get("path"):
                    continue
                kind = str(report["kind"])
                path = report_path(report, report_root)
                if not path.is_file():
                    errors.append(f"missing benchmark report: {path}")
                    continue
                errors.extend(errors_for_report(path, kind, entry))
                if kind == "late-interaction-stage-a":
                    try:
                        stage_a_hashes.add(
                            "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
                        )
                    except OSError as error:
                        errors.append(f"{path}: cannot hash report: {error}")
                elif kind == "late-interaction-stage-b":
                    try:
                        stage_b = json.loads(path.read_text(encoding="utf-8"))
                    except (OSError, json.JSONDecodeError):
                        continue
                    if isinstance(stage_b, dict):
                        stage_b_bindings.append(
                            (path, str(stage_b.get("stage_a_report_hash", "")))
                        )
        for path, referenced_hash in stage_b_bindings:
            if referenced_hash not in stage_a_hashes:
                errors.append(
                    f"{path}: stage_a_report_hash does not match a referenced Stage A report"
                )
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1
    print(f"benchmark evidence valid: {manifest}")
    if report_root is not None:
        print(f"benchmark reports valid: {report_root}")
    return 0




def parser() -> argparse.ArgumentParser:
    command_parser = argparse.ArgumentParser(description=__doc__)
    command_parser.add_argument("--manifest", type=Path, required=True)
    command_parser.add_argument("--report-root", type=Path)
    return command_parser


def main() -> int:
    args = parser().parse_args()
    return validate(args.manifest, args.report_root)


if __name__ == "__main__":
    raise SystemExit(main())
