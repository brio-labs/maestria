use crate::{i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, to_port_error, u64_to_i64};
use maestria_domain::{ScopeId, TaskId};
use maestria_ports::{
    EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, HarnessRunId, PortError,
};
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) fn record_intent(
    connection: &mut Connection,
    intent: EffectJournalIntent,
) -> Result<EffectJournalEntry, PortError> {
    let transaction = connection.transaction().map_err(to_port_error)?;
    let run_id_i64 = u64_to_i64(intent.run_id.value())?;

    let max_gen_i64: Option<i64> = transaction
        .query_row(
            "SELECT MAX(generation) FROM effect_journal WHERE run_id = ?1",
            params![run_id_i64],
            |row| row.get(0),
        )
        .map_err(to_port_error)?;

    let max_gen = optional_i64_to_u64(max_gen_i64)?;
    let next_generation = max_gen.map_or(1, |value| value.saturating_add(1));
    let generation = match intent.requested_generation {
        Some(requested) if requested >= next_generation => requested,
        _ => next_generation,
    };
    let generation_i64 = u64_to_i64(generation)?;

    if let Some(prev_gen) = max_gen_i64 {
        transaction
            .execute(
                "UPDATE effect_journal SET status = 'Superseded' \
                 WHERE run_id = ?1 AND generation = ?2 \
                 AND status IN ('Intent', 'Started')",
                params![run_id_i64, prev_gen],
            )
            .map_err(to_port_error)?;
    }

    let task_id_i64 = optional_u64_to_i64(intent.task_id.map(|t| t.value()))?;
    let scope_id_i64 = u64_to_i64(intent.scope_id.value())?;
    let requested_gen_i64 = optional_u64_to_i64(intent.requested_generation)?;
    transaction
        .execute(
            "INSERT INTO effect_journal \
             (run_id, generation, task_id, capability, command, scope_id, requested_generation, status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'Intent')",
            params![
                run_id_i64,
                generation_i64,
                task_id_i64,
                intent.capability,
                intent.command,
                scope_id_i64,
                requested_gen_i64
            ],
        )
        .map_err(to_port_error)?;
    transaction.commit().map_err(to_port_error)?;

    Ok(EffectJournalEntry {
        run_id: intent.run_id,
        task_id: intent.task_id,
        capability: intent.capability,
        command: intent.command,
        scope_id: intent.scope_id,
        generation,
        status: EffectJournalStatus::Intent,
    })
}

pub(crate) fn record_started(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: u64,
) -> Result<(), PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation)?;
    let updated = connection.execute(
        "UPDATE effect_journal SET status = 'Started' WHERE run_id = ?1 AND generation = ?2 AND status = 'Intent'",
        params![run_id_i64, generation_i64],
    ).map_err(to_port_error)?;

    if updated == 0 {
        return Err(PortError::NotFound);
    }
    Ok(())
}
pub(crate) fn claim_feedback(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: u64,
) -> Result<(), PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation)?;
    let updated = connection
        .execute(
            "UPDATE effect_journal SET status = 'FeedbackAccepted' \
             WHERE run_id = ?1 AND generation = ?2 \
             AND status IN ('Intent', 'Started')",
            params![run_id_i64, generation_i64],
        )
        .map_err(to_port_error)?;
    if updated == 0 {
        return Err(PortError::NotFound);
    }
    Ok(())
}

pub(crate) fn record_terminal(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: u64,
    status: EffectJournalStatus,
) -> Result<(), PortError> {
    let status_str = match status {
        EffectJournalStatus::Completed => "Completed",
        EffectJournalStatus::Failed => "Failed",
        EffectJournalStatus::Paused => "Paused",
        EffectJournalStatus::Superseded => "Superseded",
        _ => {
            return Err(PortError::InvalidInput {
                message: "terminal journal status required".to_string(),
            });
        }
    };

    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation)?;
    let updated = connection
        .execute(
            "UPDATE effect_journal SET status = ?1 \
             WHERE run_id = ?2 AND generation = ?3 \
             AND status IN ('Intent', 'Started', 'FeedbackAccepted')",
            params![status_str, run_id_i64, generation_i64],
        )
        .map_err(to_port_error)?;

    if updated == 0 {
        return Err(PortError::NotFound);
    }
    Ok(())
}
pub(crate) fn scan_in_flight(
    connection: &Connection,
) -> Result<Vec<EffectJournalEntry>, PortError> {
    let mut stmt = connection
        .prepare(
            "SELECT run_id, generation, task_id, capability, command, scope_id, status \
             FROM effect_journal \
             WHERE status IN ('Intent', 'Started', 'FeedbackAccepted') \
             ORDER BY run_id, generation",
        )
        .map_err(to_port_error)?;

    let entries = stmt
        .query_map([], |row| {
            let run_id_i64: i64 = row.get(0)?;
            let generation_i64: i64 = row.get(1)?;
            let task_id_i64: Option<i64> = row.get(2)?;
            let capability: String = row.get(3)?;
            let command: String = row.get(4)?;
            let scope_id_i64: i64 = row.get(5)?;
            let status_str: String = row.get(6)?;

            Ok((
                run_id_i64,
                generation_i64,
                task_id_i64,
                capability,
                command,
                scope_id_i64,
                status_str,
            ))
        })
        .map_err(to_port_error)?;

    let mut result = Vec::new();
    for row in entries {
        let (
            run_id_i64,
            generation_i64,
            task_id_i64,
            capability,
            command,
            scope_id_i64,
            status_str,
        ) = row.map_err(to_port_error)?;

        let run_id = HarnessRunId::new(i64_to_u64(run_id_i64)?);
        let generation = i64_to_u64(generation_i64)?;
        let task_id = optional_i64_to_u64(task_id_i64)?.map(TaskId::new);
        let scope_id = ScopeId::new(i64_to_u64(scope_id_i64)?);
        let status = match status_str.as_str() {
            "Intent" => EffectJournalStatus::Intent,
            "Started" => EffectJournalStatus::Started,
            "FeedbackAccepted" => EffectJournalStatus::FeedbackAccepted,
            _ => {
                return Err(PortError::Internal {
                    message: "invalid status in db".to_string(),
                });
            }
        };

        result.push(EffectJournalEntry {
            run_id,
            task_id,
            capability,
            command,
            scope_id,
            generation,
            status,
        });
    }
    Ok(result)
}

pub(crate) fn is_current(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: u64,
) -> Result<bool, PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation)?;
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM effect_journal WHERE run_id = ?1 AND generation = ?2",
            params![run_id_i64, generation_i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    Ok(matches!(
        status.as_deref(),
        Some("Intent" | "Started" | "FeedbackAccepted")
    ))
}

pub(crate) fn is_feedback_accepted(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: u64,
) -> Result<bool, PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation)?;
    let status: Option<String> = connection
        .query_row(
            "SELECT status FROM effect_journal WHERE run_id = ?1 AND generation = ?2",
            params![run_id_i64, generation_i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    Ok(status.as_deref() == Some("FeedbackAccepted"))
}
