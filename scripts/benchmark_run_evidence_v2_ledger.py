"""Attempt and campaign-consumption ledger verification for v2 evidence."""

from __future__ import annotations

import sys
from datetime import datetime
from decimal import Decimal
from typing import Any

_artifact_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_artifacts"]
_common_helpers = sys.modules["_sillage_benchmark_run_evidence_v2_common"]
VerifiedArtifact = _artifact_helpers.VerifiedArtifact
_timestamp = _artifact_helpers._timestamp
_same_json = _common_helpers._same_json
_decimal = _common_helpers._decimal

def _attempt_receipts(
    row: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    context: dict[str, Any],
) -> tuple[str | None, str | None, datetime | None, datetime | None]:
    protocol = context["protocol"]
    start_id, terminal_id = row["start_receipt_artifact_id"], row["terminal_receipt_artifact_id"]
    start_art = verified.get(start_id)
    start = start_art.content if start_art is not None else None
    frozen_at = _timestamp(context["frozen_at"])
    start_time = _timestamp(start.get("recorded_at")) if isinstance(start, dict) else None
    if (
        start_art is None
        or start_art.role != "start_receipt"
        or not isinstance(start, dict)
        or start.get("receipt_id") != start_id
        or start.get("run_id") != row["run_id"]
        or start.get("attempt_id") != row["attempt_id"]
        or start.get("protocol_id") != protocol["protocol_id"]
        or start.get("protocol_revision") != protocol["revision"]
        or str(start.get("protocol_digest", "")).lower() != context["protocol_digest"].lower()
        or start.get("freeze_id") != context["freeze_id"]
        or start.get("frozen_at") != context["frozen_at"]
        or frozen_at is None
        or start_time is None
        or start_time <= frozen_at
    ):
        return None, None, None, None
    if row["state"] == "unfinished":
        if terminal_id is None:
            return start_id, None, start_time, None
        return None, None, None, None
    terminal_art = verified.get(terminal_id)
    terminal = terminal_art.content if terminal_art is not None else None
    terminal_time = _timestamp(terminal.get("recorded_at")) if isinstance(terminal, dict) else None
    if (
        terminal_art is None
        or terminal_art.role != "terminal_receipt"
        or not isinstance(terminal, dict)
        or terminal.get("receipt_id") != terminal_id
        or terminal.get("run_id") != row["run_id"]
        or terminal.get("attempt_id") != row["attempt_id"]
        or terminal.get("protocol_id") != protocol["protocol_id"]
        or terminal.get("protocol_revision") != protocol["revision"]
        or terminal.get("status") != row["state"]
        or terminal.get("start_receipt_id") != start_id
        or terminal_time is None
        or start_time >= terminal_time
    ):
        return None, None, None, None
    return start_id, terminal_id, start_time, terminal_time


def _evaluation_attempts(
    attempts: list[dict[str, Any]],
    verified: dict[str, VerifiedArtifact],
    context: dict[str, Any],
    timeline: list[tuple[str, str, int, datetime, datetime | None]],
    errors: list[str],
) -> tuple[
    dict[str, dict[str, Any]], set[str], set[str], set[str], set[str], Decimal, Decimal
] | None:
    planned = context["planned"]
    if len(attempts) < len(planned):
        errors.append("attempt ledger omits planned first attempts")
        return None
    first: dict[str, dict[str, Any]] = {}
    slot_attempt_counts: dict[str, int] = {}
    run_ids: set[str] = set()
    attempt_ids: set[str] = set()
    starts: set[str] = set()
    terminals: set[str] = set()
    total_compute, total_cost = Decimal(0), Decimal(0)
    for index, row in enumerate(attempts):
        slot, attempt_number = row["slot_id"], row["attempt_number"]
        plan = context["plan_by_slot"].get(slot)
        if (
            plan is None
            or row["ordinal"] != index + 1
            or any(row[key] != plan[key] for key in ("case_id", "system_id", "repetition", "seed"))
            or row["run_id"] in run_ids
            or row["attempt_id"] in attempt_ids
            or attempt_number != slot_attempt_counts.get(slot, 0) + 1
            or (index < len(planned) and (attempt_number != 1 or slot != planned[index]["slot_id"]))
        ):
            errors.append("attempt ledger contains omitted, reordered, or duplicate evaluation attempts")
            return None
        if attempt_number == 1 and any(
            row[key] != plan[key] for key in ("run_id", "attempt_id")
        ):
            errors.append("first evaluation attempt identity differs from frozen protocol roster")
            return None
        run_ids.add(row["run_id"])
        attempt_ids.add(row["attempt_id"])
        slot_attempt_counts[slot] = attempt_number
        if attempt_number == 1:
            first[slot] = row
        total_compute += Decimal(str(row["compute_seconds"]))
        total_cost += Decimal(str(row["cost_amount"]))
        start_id, terminal_id, start_time, terminal_time = _attempt_receipts(
            row, verified, context
        )
        if start_id is None or start_time is None:
            errors.append("attempt ledger contains an unbound start or terminal receipt")
            return None
        starts.add(start_id)
        if terminal_id is not None:
            terminals.add(terminal_id)
        timeline.append(("evaluation", slot, attempt_number, start_time, terminal_time))
    return first, run_ids, attempt_ids, starts, terminals, total_compute, total_cost



def _campaign_consumptions(
    consumptions: list[dict[str, Any]],
    descriptors: list[dict[str, Any]],
    verified: dict[str, VerifiedArtifact],
    context: dict[str, Any],
    timeline: list[tuple[str, str, int, datetime, datetime | None]],
    run_ids: set[str],
    attempt_ids: set[str],
    starts: set[str],
    terminals: set[str],
    errors: list[str],
) -> tuple[Decimal, Decimal]:
    consumption_attempt_counts: dict[str, int] = {}
    consumption_phases: dict[str, str] = {}
    referenced_artifacts: set[str] = set()
    total_compute, total_cost = Decimal(0), Decimal(0)
    for row in consumptions:
        artifact_id = row["artifact_id"]
        artifact = verified.get(artifact_id)
        evidence = artifact.content if artifact is not None else None
        referenced_artifacts.add(artifact_id)
        count = consumption_attempt_counts.get(row["consumption_id"], 0) + 1
        phase = consumption_phases.setdefault(row["consumption_id"], row["phase"])
        expected = {key: value for key, value in row.items() if key != "artifact_id"}
        actual = (
            {key: value for key, value in evidence.items()
             if key not in {"evidence_version", "kind", "campaign_id", "protocol_digest"}}
            if isinstance(evidence, dict) else None
        )
        if (
            artifact is None
            or artifact.role != "campaign_consumption"
            or evidence.get("campaign_id") != context["campaign_id"]
            or str(evidence.get("protocol_digest", "")).lower() != context["protocol_digest"].lower()
            or not _same_json(actual, expected)
            or phase != row["phase"]
            or row["attempt_number"] != count
            or row["run_id"] in run_ids
            or row["attempt_id"] in attempt_ids
        ):
            errors.append("campaign consumption record does not bind unique setup provenance and cost")
            continue
        consumption_attempt_counts[row["consumption_id"]] = count
        run_ids.add(row["run_id"])
        attempt_ids.add(row["attempt_id"])
        total_compute += Decimal(str(row["compute_seconds"]))
        total_cost += Decimal(str(row["cost_amount"]))
        start_id, terminal_id, start_time, terminal_time = _attempt_receipts(
            row, verified, context
        )
        if start_id is None or start_time is None:
            errors.append("campaign consumption record has an unbound start or terminal receipt")
            continue
        starts.add(start_id)
        if terminal_id is not None:
            terminals.add(terminal_id)
        timeline.append((
            "consumption", row["consumption_id"], row["attempt_number"],
            start_time, terminal_time,
        ))
    if {row["phase"] for row in consumptions} != {"preparation", "tuning", "setup"}:
        errors.append("campaign ledger must account for preparation, tuning, and setup activities")
    if (
        len(referenced_artifacts) != len(consumptions)
        or referenced_artifacts != {item["artifact_id"] for item in descriptors}
    ):
        errors.append("campaign ledger contains unlinked consumption evidence")
    return total_compute, total_cost


def _attempt_order_errors(
    timeline: list[tuple[str, str, int, datetime, datetime | None]],
    context: dict[str, Any],
    errors: list[str],
) -> None:
    attempts_by_identity: dict[
        tuple[str, str], dict[int, tuple[datetime, datetime | None]]
    ] = {}
    first_starts: dict[str, datetime] = {}
    events: list[tuple[datetime, int]] = []
    for kind, identity, number, start_time, terminal_time in timeline:
        previous = attempts_by_identity.setdefault((kind, identity), {})
        if number > 1:
            predecessor = previous.get(number - 1)
            if predecessor is None or predecessor[1] is None or start_time <= predecessor[1]:
                errors.append("retry starts before its predecessor terminal receipt")
        previous[number] = (start_time, terminal_time)
        if kind == "evaluation" and number == 1:
            first_starts[identity] = start_time
        events.append((start_time, 1))
        if terminal_time is not None:
            events.append((terminal_time, -1))

    planned = context["planned"]
    if len(first_starts) == len(planned):
        prior_start = first_starts[planned[0]["slot_id"]]
        for row in planned[1:]:
            current_start = first_starts[row["slot_id"]]
            if current_start <= prior_start:
                errors.append("first-attempt receipt starts do not follow frozen protocol order")
                break
            prior_start = current_start

    active = 0
    maximum = context["protocol"]["max_concurrent_attempts"]
    for _, change in sorted(events):
        active += change
        if active > maximum:
            errors.append("attempt receipt intervals exceed the frozen concurrency limit")
            break


 
def _ledger_evidence_errors(
    payload: dict[str, Any],
    verified: dict[str, VerifiedArtifact],
    context: dict[str, Any],
    errors: list[str],
) -> tuple[dict[str, dict[str, Any]], list[dict[str, Any]]]:
    required, records = context["required"], context["records"]
    ledger_descriptor = required["attempt_ledger"]
    ledger = records["attempt_ledger"]
    budget, declaration = payload["budgets"], records["declared_budget"]
    budget_digest = verified[required["declared_budget"]["artifact_id"]].digest
    if (
        payload["execution"]["order_artifact"] != ledger_descriptor["artifact_id"]
        or ledger["campaign_id"] != context["campaign_id"]
        or ledger["protocol_digest"] != context["protocol_digest"]
        or ledger["case_list_digest"] != payload["data"]["case_list_digest"]
        or ledger["declared_budget_digest"] != budget_digest
        or ledger["currency"] != budget["currency"]
        or ledger["first_attempt_count"] != len(context["planned"])
    ):
        errors.append("attempt ledger does not bind frozen roster and declared budget")
    attempts = ledger["attempts"]
    timeline: list[tuple[str, str, int, datetime, datetime | None]] = []
    evaluation = _evaluation_attempts(attempts, verified, context, timeline, errors)
    if evaluation is None:
        return {}, []
    first, run_ids, attempt_ids, starts, terminals, total_compute, total_cost = evaluation
    consumptions = ledger["consumptions"]
    descriptors = context["roles"].get("campaign_consumption", [])
    consumption_compute, consumption_cost = _campaign_consumptions(
        consumptions, descriptors, verified, context, timeline, run_ids, attempt_ids,
        starts, terminals, errors,
    )
    _attempt_order_errors(timeline, context, errors)
    total_compute += consumption_compute
    total_cost += consumption_cost
    if (
        set(first) != set(context["plan_by_slot"])
        or starts != {item["artifact_id"] for item in context["roles"].get("start_receipt", [])}
        or terminals != {item["artifact_id"] for item in context["roles"].get("terminal_receipt", [])}
    ):
        errors.append("campaign ledger omits attempts or contains unlinked receipts")
    if (
        Decimal(str(ledger["total_compute_seconds"])) != total_compute
        or Decimal(str(ledger["total_cost_amount"])) != total_cost
        or _decimal(budget["actual_compute_seconds"]) != total_compute
        or _decimal(budget["actual_cost_amount"]) != total_cost
    ):
        errors.append("campaign budget omits or alters attempted consumption")
    if (
        total_compute > Decimal(str(declaration["max_compute_seconds"]))
        or total_cost > Decimal(str(declaration["max_cost_amount"]))
    ):
        errors.append("campaign consumption exceeds the declared budget")
    current = [row for row in attempts if (
        row["run_id"] == payload["run_id"] and row["attempt_id"] == payload["attempt_id"]
    )]
    if len(current) != 1 or current[0]["attempt_number"] != 1 or current[0]["state"] != "complete":
        errors.append("manifest is not a complete first evaluation attempt in the full ledger")
    else:
        for key, artifact_id in (
            ("start_receipt", current[0]["start_receipt_artifact_id"]),
            ("terminal_receipt", current[0]["terminal_receipt_artifact_id"]),
        ):
            inline, artifact = payload["lifecycle"][key], verified[artifact_id]
            content = {name: value for name, value in artifact.content.items()
                       if name not in {"kind", "evidence_version"}}
            if (
                inline["digest"].lower() != artifact.digest.lower()
                or not _same_json(content, {
                    name: value for name, value in inline.items() if name != "digest"
                })
            ):
                errors.append("manifest receipt differs from its verified receipt artifact")
    return first, consumptions
