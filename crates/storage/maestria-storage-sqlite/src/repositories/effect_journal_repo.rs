use crate::sqlite_store::{
    i64_to_u64, optional_i64_to_u64, optional_u64_to_i64, to_port_error, u64_to_i64,
};
use maestria_domain::{BlobId, HarnessRunId, JournalGeneration, ScopeId, TaskId};
use maestria_ports::{
    EffectJournalEntry, EffectJournalIntent, EffectJournalStatus, HarnessOutcome, PortError,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct StoredHarnessOutcome {
    run_id: u64,
    command: String,
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    duration_secs: u64,
    duration_nanos: u32,
    artifacts_created: Vec<u64>,
    diff_summary: Option<String>,
    validation_hints: Vec<String>,
}

impl StoredHarnessOutcome {
    fn from_domain(outcome: &HarnessOutcome) -> Self {
        Self {
            run_id: outcome.run_id.value(),
            command: outcome.command.clone(),
            exit_code: outcome.exit_code,
            stdout: outcome.stdout.clone(),
            stderr: outcome.stderr.clone(),
            duration_secs: outcome.duration.as_secs(),
            duration_nanos: outcome.duration.subsec_nanos(),
            artifacts_created: outcome
                .artifacts_created
                .iter()
                .map(BlobId::value)
                .collect(),
            diff_summary: outcome.diff_summary.clone(),
            validation_hints: outcome.validation_hints.clone(),
        }
    }

    fn into_domain(self) -> HarnessOutcome {
        HarnessOutcome {
            run_id: HarnessRunId::new(self.run_id),
            command: self.command,
            exit_code: self.exit_code,
            stdout: self.stdout,
            stderr: self.stderr,
            duration: std::time::Duration::new(self.duration_secs, self.duration_nanos),
            artifacts_created: self
                .artifacts_created
                .into_iter()
                .map(BlobId::new)
                .collect(),
            diff_summary: self.diff_summary,
            validation_hints: self.validation_hints,
        }
    }
}

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
    let next_generation =
        JournalGeneration::new(max_gen.map_or(1, |value| value.saturating_add(1)));
    let generation = match intent.requested_generation {
        Some(requested) if requested.value() >= next_generation.value() => requested,
        _ => next_generation,
    };
    let generation_i64 = u64_to_i64(generation.value())?;

    if let Some(prev_gen) = max_gen_i64 {
        transaction
            .execute(
                "UPDATE effect_journal SET status = 'Superseded' \
                 WHERE run_id = ?1 AND generation = ?2 \
                 AND status IN ('Intent', 'Started', 'FeedbackAccepted')",
                params![run_id_i64, prev_gen],
            )
            .map_err(to_port_error)?;
    }

    let task_id_i64 = optional_u64_to_i64(intent.task_id.map(|t| t.value()))?;
    let scope_id_i64 = u64_to_i64(intent.scope_id.value())?;
    let requested_gen_i64 = optional_u64_to_i64(
        intent
            .requested_generation
            .map(|generation| generation.value()),
    )?;
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
        feedback: None,
    })
}

pub(crate) fn record_started(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: maestria_domain::JournalGeneration,
) -> Result<(), PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation.value())?;
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
    generation: maestria_domain::JournalGeneration,
) -> Result<(), PortError> {
    claim_feedback_with_outcome(connection, run_id, generation, None)
}

pub(crate) fn claim_feedback_with_outcome(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: maestria_domain::JournalGeneration,
    outcome: Option<&HarnessOutcome>,
) -> Result<(), PortError> {
    let transaction = connection.unchecked_transaction().map_err(to_port_error)?;
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation.value())?;
    let feedback = outcome
        .map(StoredHarnessOutcome::from_domain)
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| PortError::internal("encode harness feedback", error.to_string()))?;
    let updated = transaction
        .execute(
            "UPDATE effect_journal SET status = 'FeedbackAccepted', feedback_json = ?3 \
             WHERE run_id = ?1 AND generation = ?2 \
             AND status IN ('Intent', 'Started')",
            params![run_id_i64, generation_i64, feedback],
        )
        .map_err(to_port_error)?;
    if updated == 0 {
        return Err(PortError::NotFound);
    }
    transaction.commit().map_err(to_port_error)
}
pub(crate) fn feedback_outcome(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: maestria_domain::JournalGeneration,
) -> Result<Option<HarnessOutcome>, PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation.value())?;
    let feedback_json: Option<String> = connection
        .query_row(
            "SELECT feedback_json FROM effect_journal WHERE run_id = ?1 AND generation = ?2",
            params![run_id_i64, generation_i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?
        .flatten();
    feedback_json
        .as_deref()
        .map(serde_json::from_str::<StoredHarnessOutcome>)
        .transpose()
        .map_err(|error| PortError::internal("decode harness feedback", error.to_string()))
        .map(|outcome| outcome.map(StoredHarnessOutcome::into_domain))
}

pub(crate) fn record_terminal(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: maestria_domain::JournalGeneration,
    status: EffectJournalStatus,
) -> Result<(), PortError> {
    let status_str = match status {
        EffectJournalStatus::Completed => "Completed",
        EffectJournalStatus::Failed => "Failed",
        EffectJournalStatus::Paused => "Paused",
        EffectJournalStatus::Superseded => "Superseded",
        _ => {
            return Err(PortError::InvalidInputContext {
                context: "terminal journal status required",
                source: "status must be terminal".to_string(),
            });
        }
    };
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation.value())?;
    let updated = connection
        .execute(
            "UPDATE effect_journal SET status = ?1 \
             WHERE run_id = ?2 AND generation = ?3 \
             AND (status IN ('Intent', 'Started', 'FeedbackAccepted') OR status = ?1)",
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
    let mut stmt = connection.prepare_cached("SELECT run_id, generation, task_id, capability, command, scope_id, status, feedback_json \
     FROM effect_journal \
     WHERE status IN ('Intent', 'Started', 'FeedbackAccepted') \
     ORDER BY run_id, generation")
        .map_err(to_port_error)?;
    let entries = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
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
            feedback_json,
        ) = row.map_err(to_port_error)?;
        let status = match status_str.as_str() {
            "Intent" => EffectJournalStatus::Intent,
            "Started" => EffectJournalStatus::Started,
            "FeedbackAccepted" => EffectJournalStatus::FeedbackAccepted,
            _ => {
                return Err(PortError::InternalContext {
                    context: "decode effect journal status",
                    source: "invalid status in db".to_string(),
                });
            }
        };
        let feedback = feedback_json
            .as_deref()
            .map(serde_json::from_str::<StoredHarnessOutcome>)
            .transpose()
            .map_err(|error| PortError::internal("decode harness feedback", error.to_string()))?
            .map(StoredHarnessOutcome::into_domain);
        result.push(EffectJournalEntry {
            run_id: HarnessRunId::new(i64_to_u64(run_id_i64)?),
            task_id: optional_i64_to_u64(task_id_i64)?.map(TaskId::new),
            capability,
            command,
            scope_id: ScopeId::new(i64_to_u64(scope_id_i64)?),
            generation: JournalGeneration::new(i64_to_u64(generation_i64)?),
            status,
            feedback,
        });
    }
    Ok(result)
}

pub(crate) fn is_current(
    connection: &Connection,
    run_id: HarnessRunId,
    generation: maestria_domain::JournalGeneration,
) -> Result<bool, PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation.value())?;
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
    generation: maestria_domain::JournalGeneration,
) -> Result<bool, PortError> {
    let run_id_i64 = u64_to_i64(run_id.value())?;
    let generation_i64 = u64_to_i64(generation.value())?;
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
