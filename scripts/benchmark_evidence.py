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
REQUIRED_RESULT_KEYS = ("quality", "resource", "security")
REQUIRED_ENVIRONMENT_KEYS = ("os", "rust_toolchain", "cpu_arch")


def is_non_negative_int(value: Any) -> bool:
    return type(value) is int and value >= 0


def is_valid_measurement_status(value: Any) -> bool:
    if value == "Measured":
        return True
    if not isinstance(value, dict):
        return False
    unavailable = value.get("Unavailable")
    if not isinstance(unavailable, dict):
        return False
    reason = unavailable.get("reason")
    return isinstance(reason, str) and bool(reason.strip())


def is_valid_provider_status(value: Any) -> bool:
    if value == "Available":
        return True
    if not isinstance(value, dict) or len(value) != 1:
        return False
    state, details = next(iter(value.items()))
    if state not in {"Degraded", "Unavailable"} or not isinstance(details, dict):
        return False
    reason = details.get("reason")
    return isinstance(reason, str) and bool(reason.strip())


def is_valid_visual_provider_config(route: Any, value: Any) -> bool:
    if not isinstance(value, dict):
        return False
    expected = {
        "TextLayout": {
            "model": "page-text-layout-v1+rapidocr-onnxruntime-1.4.4",
            "provider": "text-layout+rapidocr-onnxruntime",
        },
        "Visual": {
            "execution_mode": "sequential",
            "inter_op_threads": 1,
            "intra_op_threads": 4,
            "model": "siglip-base-patch16-224",
            "onnxruntime": "1.30.0",
            "provider": "siglip-onnx",
        },
    }.get(route)
    return expected is not None and all(
        value.get(key) == expected_value
        for key, expected_value in expected.items()
    )


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

    for key in ("measurement_kind", "evaluation_date"):
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
                    if not is_non_negative_int(observation.get(key)):
                        errors.append(f"{prefix}.{key} must be a non-negative integer")
                if not isinstance(observation.get("outcome_correct"), bool):
                    errors.append(f"{prefix}.outcome_correct must be boolean")
                if not is_valid_measurement_status(observation.get("measurement_status")):
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
                    if not is_non_negative_int(size.get(key)):
                        errors.append(f"{prefix}.{key} must be a non-negative integer")
                if not isinstance(size.get("measurements_ms"), list) or not size[
                    "measurements_ms"
                ]:
                    errors.append(f"{prefix}.measurements_ms must be non-empty")
                elif any(
                    not is_non_negative_int(value) for value in size["measurements_ms"]
                ):
                    errors.append(f"{prefix}.measurements_ms must be non-negative integers")
    elif kind == "visual":
        if report.get("provider_status") != "unavailable":
            errors.append(f"{path}: visual report must state provider_status=unavailable")
        observations = report.get("observations")
        if not isinstance(observations, list) or not observations:
            errors.append(f"{path}: observations must be non-empty")
        else:
            for index, observation in enumerate(observations):
                prefix = f"{path}: observations[{index}]"
                if not isinstance(observation, dict):
                    errors.append(f"{prefix} must be an object")
                    continue
                for key in ("case_id", "route", "measurement_status", "provider_status"):
                    if key not in observation:
                        errors.append(f"{prefix} missing {key}")
                if observation.get("route") not in {"TextLayout", "Visual"}:
                    errors.append(f"{prefix}.route is invalid")
                if not is_valid_provider_status(observation.get("provider_status")):
                    errors.append(f"{prefix}.provider_status is invalid")
                if not is_valid_measurement_status(observation.get("measurement_status")):
                    errors.append(f"{prefix}.measurement_status is invalid")
    elif kind == "visual-provider":
        for key in ("measurement_kind", "evaluation_date", "corpus_id", "corpus_revision"):
            if not str(report.get(key, "")).strip():
                errors.append(f"{path}: missing {key}")
        if manifest_entry is not None:
            corpus = manifest_entry.get("corpus", {})
            for key, expected in (
                ("corpus_id", corpus.get("id")),
                ("corpus_revision", corpus.get("revision")),
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
                for key in (
                    "case_id",
                    "route",
                    "measurement_status",
                    "corpus_id",
                    "corpus_revision",
                    "model_fingerprint",
                    "provider_config",
                    "provider_status",
                ):
                    if key not in observation:
                        errors.append(f"{prefix} missing {key}")
                if observation.get("route") not in {"TextLayout", "Visual"}:
                    errors.append(f"{prefix}.route is invalid")
                provider_status = observation.get("provider_status")
                if not is_valid_provider_status(provider_status):
                    errors.append(f"{prefix}.provider_status is invalid")
                elif observation.get("route") == "Visual" and provider_status != "Available":
                    errors.append(f"{prefix}.visual provider_status is not Available")
                if observation.get("corpus_id") != report.get("corpus_id"):
                    errors.append(f"{prefix}.corpus_id is not bound to report")
                if observation.get("corpus_revision") != report.get("corpus_revision"):
                    errors.append(f"{prefix}.corpus_revision is not bound to report")
                if not isinstance(observation.get("model_fingerprint"), str) or not str(
                    observation.get("model_fingerprint")
                ).strip():
                    errors.append(f"{prefix}.model_fingerprint is invalid")
                if not is_valid_visual_provider_config(
                    observation.get("route"), observation.get("provider_config")
                ):
                    errors.append(f"{prefix}.provider_config is invalid")
                for key in (
                    "latency_ms",
                    "memory_bytes",
                    "disk_bytes",
                    "energy_millijoules",
                    "privacy_violations",
                    "security_violations",
                ):
                    if not is_non_negative_int(observation.get(key)):
                        errors.append(f"{prefix}.{key} must be a non-negative integer")
                if not is_valid_measurement_status(observation.get("measurement_status")):
                    errors.append(f"{prefix}.measurement_status is invalid")
        winning_classes = report.get("winning_classes")
        if not isinstance(winning_classes, list):
            errors.append(f"{path}: winning_classes must be a list")
        else:
            allowed_classes = {"Text", "Table", "Chart", "Figure", "Formula", "ScannedPage"}
            valid_classes = all(
                isinstance(class_name, str) and class_name in allowed_classes
                for class_name in winning_classes
            )
            if not valid_classes:
                errors.append(f"{path}: winning_classes contains an invalid class")
            elif len(set(winning_classes)) != len(winning_classes):
                errors.append(f"{path}: winning_classes must not contain duplicates")
            if winning_classes and observations and any(
                not (
                    isinstance(observation, dict)
                    and observation.get("measurement_status") == "Measured"
                )
                for observation in observations
            ):
                errors.append(
                    f"{path}: unavailable measurements cannot authorize winning_classes"
                )
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
                if not is_valid_measurement_status(observation.get("measurement_status")):
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
        for entry in payload.get("benchmarks", []):
            if not isinstance(entry, dict):
                continue
            for report in entry.get("reports", []):
                if not isinstance(report, dict) or not report.get("kind") or not report.get("path"):
                    continue
                path = report_path(report, report_root)
                if not path.is_file():
                    errors.append(f"missing benchmark report: {path}")
                else:
                    errors.extend(
                        errors_for_report(path, str(report["kind"]), entry)
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
