//! Startup recovery queue staging: the order inputs are admitted into the runtime and the
//! event-log identities that seed watcher resume tracking.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use maestria_domain::{ArtifactId, DomainEvent, DomainInput, TaskId};
use maestria_ports::{EventFilter, EventLog};
use maestria_storage_sqlite::SqliteStore;

use crate::recovery_inputs::RecoveryInputs;

/// Staging order for recovery inputs queued by the shared lifecycle.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RecoveryQueueStage {
    ResumeParser,
    FullText,
    Validation,
}

impl std::fmt::Display for RecoveryQueueStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::ResumeParser => "resume parser",
            Self::FullText => "restart full-text index",
            Self::Validation => "task validation",
        };
        formatter.write_str(label)
    }
}

/// Queue one recovery group into the runtime, draining `inputs` in order.
///
/// A failed or cancelled queue attempt leaves every input not yet accepted by the channel in
/// `inputs`, so callers can retry without losing or duplicating work.
pub(crate) async fn queue_recovery_inputs(
    runtime: &maestria_runtime::RuntimeHandle,
    inputs: &mut Vec<DomainInput>,
    stage: RecoveryQueueStage,
) -> Result<()> {
    while !inputs.is_empty() {
        let permit = runtime
            .reserve_submission()
            .await
            .with_context(|| format!("failed to reserve {stage} recovery submission"))?;
        let input = inputs.remove(0);
        permit
            .submit(input)
            .await
            .with_context(|| format!("failed to apply {stage} recovery input"))?;
    }
    Ok(())
}

/// Scan the event log for parser-resume identities, keyed by canonical source path.
pub(crate) fn source_artifact_ids(
    store: &SqliteStore,
) -> Result<BTreeMap<String, (ArtifactId, String)>> {
    let mut identities = BTreeMap::new();
    for envelope in store.scan(EventFilter { artifact_id: None })? {
        if let DomainEvent::ParserStarted {
            artifact_id,
            source_path,
            content_hash,
            ..
        } = envelope.event
        {
            let key = match std::path::Path::new(&source_path).canonicalize() {
                Ok(path) => path.display().to_string(),
                Err(_) => source_path,
            };
            identities.insert(key, (artifact_id, content_hash));
        }
    }
    Ok(identities)
}

/// Artifact ids queued for recovery in dependency order: resume parsers, then full-text rebuilds.
pub(crate) fn recovery_artifact_ids(recovery: &RecoveryInputs) -> Vec<ArtifactId> {
    recovery
        .resume_parsers
        .iter()
        .filter_map(|input| match input {
            DomainInput::ResumeParser(record) => Some(record.artifact_id),
            _ => None,
        })
        .chain(
            recovery
                .start_full_text
                .iter()
                .filter_map(|input| match input {
                    DomainInput::StartFullTextIndex(request) => Some(request.artifact_id),
                    _ => None,
                }),
        )
        .collect()
}

/// Task ids queued for validation recovery.
pub(crate) fn validation_task_ids(recovery: &RecoveryInputs) -> Vec<TaskId> {
    recovery
        .run_validations
        .iter()
        .filter_map(|input| match input {
            DomainInput::RequestTaskValidation(request) => Some(request.task_id),
            _ => None,
        })
        .collect()
}
