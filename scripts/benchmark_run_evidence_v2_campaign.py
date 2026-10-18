"""Campaign evidence facade for v2 ledgers and measurements."""

from __future__ import annotations

import sys
from typing import Any, Callable

_artifact_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_artifacts"]
_ledger_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_ledger"]
_measurement_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_measurements"]
VerifiedArtifact = _artifact_helpers.VerifiedArtifact
_schema_errors = _artifact_helpers._schema_errors
_ledger_evidence_errors = _ledger_helpers._ledger_evidence_errors
_observation_evidence_errors = _measurement_helpers._observation_evidence_errors

def _correction_evidence_errors(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    roles: dict[str, list[dict[str, Any]]],
    schema: dict[str, Any],
    validate_manifest_semantics: Callable[[dict[str, Any]], list[str]],
    errors: list[str],
) -> None:
    correction = payload["corrections"]
    originals = roles.get("original_record", [])
    if correction["supersedes_run_id"] is None:
        if originals:
            errors.append("unlinked original-record artifact is not allowed")
        return
    refs = [item for item in payload["prior_evidence_refs"]
            if item["kind"] == "run"
            and item["run_id"] == correction["supersedes_run_id"]
            and item["attempt_id"] == correction["supersedes_attempt_id"]]
    if len(originals) != 1 or len(refs) != 1:
        errors.append("correction requires one verified original record and reference")
        return
    descriptor = originals[0]
    artifact = verified[descriptor["artifact_id"]]
    original, ref = artifact.content, refs[0]
    if (
        not isinstance(original, dict)
        or type(original.get("schema_version")) is not int
        or original.get("schema_version") != 2
        or original.get("record_kind") != "benchmark_run_manifest"
        or original.get("run_id") != correction["supersedes_run_id"]
        or original.get("attempt_id") != correction["supersedes_attempt_id"]
        or original.get("run_id") == payload["run_id"]
        or _schema_errors(original, schema, schema)
        or validate_manifest_semantics(original)
        or ref["artifact_id"] != descriptor["artifact_id"]
        or ref["path"] != descriptor["path"]
        or str(ref["sha256"]).lower() != artifact.digest.lower()
    ):
        errors.append("correction reference does not bind the immutable original run bytes")
