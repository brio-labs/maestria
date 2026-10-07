"""Content-bound campaign and measurement evidence validation for v2 runs."""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path
from typing import Any, Callable


def _load_artifact_helpers() -> Any:
    module_name = "_sillage_benchmark_run_evidence_v2_artifacts"
    loaded = sys.modules.get(module_name)
    if loaded is not None:
        return loaded
    module_path = Path(__file__).with_name("benchmark_run_evidence_v2_artifacts.py")
    spec = importlib.util.spec_from_file_location(module_name, module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load v2 artifact evidence helpers")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        del sys.modules[module_name]
        raise
    return module


_artifact_helpers = _load_artifact_helpers()
VerifiedArtifact = _artifact_helpers.VerifiedArtifact
_protocol_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_protocol"]
_campaign_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_campaign"]
def _content_evidence_errors(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    schema: dict[str, Any],
    validate_manifest_semantics: Callable[[dict[str, Any]], list[str]],
) -> list[str]:
    try:
        errors: list[str] = []
        roles = _protocol_helpers._content_role_index(payload, verified, schema, errors)
        if errors:
            return errors
        context = _protocol_helpers._protocol_context(payload, verified, roles, errors)
        if context is None:
            return errors
        _protocol_helpers._run_artifact_binding_errors(payload, context, errors)
        first, consumptions = _campaign_helpers._ledger_evidence_errors(
            payload, verified, context, errors
        )
        if set(first) == set(context["plan_by_slot"]):
            _campaign_helpers._observation_evidence_errors(
                payload, verified, context, first, consumptions, errors
            )
        else:
            errors.append("attempt ledger does not provide every first attempt")
        _campaign_helpers._correction_evidence_errors(
            payload, verified, roles, schema, validate_manifest_semantics, errors
        )
        return list(dict.fromkeys(errors))
    except (ArithmeticError, AttributeError, IndexError, KeyError, RecursionError, TypeError, ValueError):
        return ["claim_eligibility: verified evidence is malformed or does not satisfy its contract"]
