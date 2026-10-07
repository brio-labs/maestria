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
    view_source: dict[str, Any] | None = None,
    allow_noncomplete: bool = False,
) -> list[str]:
    if view_source is not None:
        view_source.update({
            "context": None,
            "first": {},
            "consumptions": [],
            "content_errors": [],
            "phase_errors": {},
        })
    try:
        errors: list[str] = []
        phase_errors: dict[str, list[str]] = {}
        roles = _protocol_helpers._content_role_index(payload, verified, schema, errors)
        phase_errors["roles"] = list(errors)
        if errors:
            return errors
        before = len(errors)
        context = _protocol_helpers._protocol_context(payload, verified, roles, errors)
        phase_errors["protocol"] = errors[before:]
        if view_source is not None:
            view_source["context"] = context
            view_source["phase_errors"] = phase_errors
        if context is None:
            return errors
        before = len(errors)
        _protocol_helpers._run_artifact_binding_errors(payload, context, errors)
        phase_errors["bindings"] = errors[before:]
        before = len(errors)
        first, consumptions = _campaign_helpers._ledger_evidence_errors(
            payload, verified, context, errors, allow_noncomplete=allow_noncomplete
        )
        phase_errors["ledger"] = errors[before:]
        if view_source is not None:
            view_source.update({
                "context": context,
                "first": first,
                "consumptions": consumptions,
                "phase_errors": phase_errors,
            })
        if set(first) == set(context["plan_by_slot"]):
            before = len(errors)
            _campaign_helpers._observation_evidence_errors(
                payload, verified, context, first, consumptions, errors
            )
            phase_errors["observations"] = errors[before:]
        else:
            before = len(errors)
            errors.append("attempt ledger does not provide every first attempt")
            phase_errors["observations"] = errors[before:]
        before = len(errors)
        _campaign_helpers._correction_evidence_errors(
            payload, verified, roles, schema, validate_manifest_semantics, errors
        )
        phase_errors["corrections"] = errors[before:]
        unique_errors = list(dict.fromkeys(errors))
        if view_source is not None:
            view_source["content_errors"] = unique_errors
            view_source["phase_errors"] = phase_errors
        return unique_errors
    except (ArithmeticError, AttributeError, IndexError, KeyError, RecursionError, TypeError, ValueError):
        errors = ["claim_eligibility: verified evidence is malformed or does not satisfy its contract"]
        if view_source is not None:
            view_source["content_errors"] = errors
            view_source["phase_errors"] = {"unknown": errors}
        return errors
