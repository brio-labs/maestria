#!/usr/bin/env python3
"""Validate Maestria's checked-in benchmark evidence ledger and run reports."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
ALLOWED_STAGES = {
    "planned",
    "implementation-complete",
    "benchmark-complete",
    "product-complete",
    "released",
}
ALLOWED_FIDELITY = {"real", "synthetic", "mixed", "staged"}
ALLOWED_STATUS = {"pass", "warning", "fail", "pending", "n/a"}
REQUIRED_MILESTONES = (
    "v0.4 — Deterministic Search Baseline",
    "v0.5 — Evaluated Hybrid Retrieval",
    "v0.7 — Repository Intelligence",
    "v0.8 — Visual Document Retrieval",
)
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

    entries = payload.get("milestones")
    if not isinstance(entries, list):
        return errors + ["milestones must be a list"]
    observed = [entry.get("milestone") for entry in entries if isinstance(entry, dict)]
    if tuple(observed) != REQUIRED_MILESTONES:
        errors.append(f"milestones must be exactly {REQUIRED_MILESTONES!r}")

    for index, entry in enumerate(entries):
        prefix = f"milestones[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{prefix} must be an object")
            continue
        stage = entry.get("release_stage")
        fidelity = entry.get("data_fidelity")
        if stage not in ALLOWED_STAGES:
            errors.append(f"{prefix}.release_stage is invalid: {stage!r}")
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

        if stage in {"product-complete", "released"}:
            if fidelity != "real":
                errors.append(f"{prefix}: product stages require real data_fidelity")
            result_map = results if isinstance(results, dict) else {}
            if any(
                not isinstance(result_map.get(key), dict)
                or result_map[key].get("status") != "pass"
                for key in REQUIRED_RESULT_KEYS
            ):
                errors.append(f"{prefix}: product stages require passing results")

    return errors


def errors_for_report(path: Path, kind: str) -> list[str]:
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
        observations = report.get("observations")
        if not isinstance(observations, list) or not observations:
            errors.append(f"{path}: observations must be non-empty")
        else:
            for index, observation in enumerate(observations):
                if not isinstance(observation, dict):
                    errors.append(f"{path}: observations[{index}] must be an object")
                    continue
                for key in ("case_id", "route", "latency_ms", "outcome_correct"):
                    if key not in observation:
                        errors.append(f"{path}: observations[{index}] missing {key}")
                if "measurement_status" not in observation:
                    errors.append(f"{path}: observations[{index}] missing measurement_status")
    elif kind == "visual":
        if report.get("provider_status") != "unavailable":
            errors.append(f"{path}: visual report must state provider_status=unavailable")
        if not isinstance(report.get("observations"), list) or not report["observations"]:
            errors.append(f"{path}: observations must be non-empty")
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
        for entry in payload.get("milestones", []):
            if not isinstance(entry, dict):
                continue
            for report in entry.get("reports", []):
                if not isinstance(report, dict) or not report.get("kind") or not report.get("path"):
                    continue
                path = report_path(report, report_root)
                if not path.is_file():
                    errors.append(f"missing benchmark report: {path}")
                else:
                    errors.extend(errors_for_report(path, str(report["kind"])))
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
