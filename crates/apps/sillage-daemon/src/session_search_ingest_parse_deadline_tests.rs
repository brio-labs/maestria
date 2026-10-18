use std::time::Duration;
#[cfg(unix)]
use std::{future::Future, sync::Arc};

use anyhow::{Context, Result, bail};
use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};
#[cfg(unix)]
use tokio_util::sync::CancellationToken;

use super::{PARSE_TEST_SERIAL, ParseCheckpointGate, Workspace, clear_gate, install_gate};
use crate::{SessionOperationSettlement, SessionPreparationOwner, SessionSearchCancellation};

#[tokio::test]
async fn public_prepare_preserves_bounded_parser_deadline_cause_at_checkpoint() -> Result<()> {
    let _serial = PARSE_TEST_SERIAL.lock().await;
    let (workspace, source) = Workspace::create()?;
    let owner = SessionPreparationOwner::new();
    let defaults = SourceResourceBudget::session_default();
    let budget = SourceResourceBudget::try_new(
        defaults.max_entries(),
        defaults.max_file_bytes(),
        defaults.max_total_source_bytes(),
        defaults.max_in_memory_bytes(),
        defaults.max_results(),
        defaults.max_work_units(),
        5_000,
    )?;
    let choice = SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source.clone())?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget,
    })?;
    let (gate, reached_signal) = ParseCheckpointGate::new(source.clone());
    install_gate(gate.clone())?;
    let worker_owner = owner.clone();
    let preparation = tokio::spawn(async move {
        worker_owner
            .prepare_cancellable(choice, SessionSearchCancellation::new())
            .await
    });
    let reached = tokio::time::timeout(Duration::from_secs(20), reached_signal)
        .await
        .is_ok_and(|result| result.is_ok());
    if reached {
        tokio::time::sleep(Duration::from_millis(
            u64::from(budget.max_duration_ms()).saturating_add(50),
        ))
        .await;
    }
    gate.release();
    let joined = preparation
        .await
        .map_err(anyhow::Error::new)
        .context("deadline-bounded preparation caller failed to join");
    clear_gate(&gate);
    let preparation = joined?;
    let classified = matches!(
        &preparation,
        Err(error)
            if SessionOperationSettlement::classify_error(error)
                == Some(SessionOperationSettlement::DeadlineExceeded)
    );
    let parser_cause = matches!(
        &preparation,
        Err(error)
            if error.chain().any(|cause| {
                matches!(
                    cause.downcast_ref::<sillage_ports::BoundedParseError>(),
                    Some(sillage_ports::BoundedParseError::Budget(
                        sillage_ports::ParseBudgetFailure::DeadlineExceeded
                    ))
                )
            })
    );
    let runtime_retirement = match &preparation {
        Ok(runtime) => runtime.retire().await,
        Err(_) => Ok(()),
    };
    let owner_retirement = owner.retire().await;
    if !reached || !classified || !parser_cause {
        bail!(
            "bounded parser deadline cause failed (real parser checkpoint reached: {reached}; \
             public typed deadline classification: {classified}; exact parser cause retained: \
             {parser_cause})"
        );
    }
    runtime_retirement.context("unexpectedly prepared runtime could not retire")?;
    owner_retirement.context("deadline test owner could not retire")?;
    drop(workspace);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn selected_prepare_deadline_survives_later_parser_cancellation() -> Result<()> {
    let _serial = PARSE_TEST_SERIAL.lock().await;
    let (workspace, source) = Workspace::create()?;
    let defaults = SourceResourceBudget::session_default();
    let budget = SourceResourceBudget::try_new(
        defaults.max_entries(),
        defaults.max_file_bytes(),
        defaults.max_total_source_bytes(),
        defaults.max_in_memory_bytes(),
        defaults.max_results(),
        defaults.max_work_units(),
        100,
    )?;
    let choice = SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source.clone())?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget,
    })?;
    let memory = Arc::new(sillage_search_tantivy::RamMemoryBudget::new(
        budget.max_in_memory_bytes(),
    ));
    let source_lease = crate::session_search::source_admission::SourceLease::acquire(
        &source,
        SourceAvailability::Enabled,
        &memory,
    )?;
    let cancellation = CancellationToken::new();
    let (gate, reached_signal) = ParseCheckpointGate::new(source.clone());
    install_gate(gate.clone())?;
    let mut preparation = Box::pin(crate::SessionSearchRuntime::prepare_native(
        choice,
        cancellation.clone(),
        None,
        source_lease,
        memory,
    ));
    let initial_poll =
        std::future::poll_fn(|context| std::task::Poll::Ready(preparation.as_mut().poll(context)))
            .await;
    let first_poll_waiting = matches!(&initial_poll, std::task::Poll::Pending);
    let reached = if first_poll_waiting {
        tokio::time::timeout(Duration::from_secs(10), reached_signal)
            .await
            .is_ok_and(|result| result.is_ok())
    } else {
        false
    };
    if reached {
        tokio::time::sleep(Duration::from_millis(
            u64::from(budget.max_duration_ms()).saturating_add(50),
        ))
        .await;
    }
    let selected_deadline_wait_pending = if reached {
        Some(
            std::future::poll_fn(|context| {
                std::task::Poll::Ready(preparation.as_mut().poll(context))
            })
            .await,
        )
    } else {
        None
    };
    cancellation.cancel();
    gate.release();
    let preparation_result = match initial_poll {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => preparation.await,
    };
    clear_gate(&gate);
    let classified = preparation_result.as_ref().err().is_some_and(|error| {
        SessionOperationSettlement::classify_error(error)
            == Some(SessionOperationSettlement::DeadlineExceeded)
    });
    let parser_cancel_cause = preparation_result.as_ref().err().is_some_and(|error| {
        error.chain().any(|cause| {
            matches!(
                cause.downcast_ref::<sillage_ports::BoundedParseError>(),
                Some(sillage_ports::BoundedParseError::Budget(
                    sillage_ports::ParseBudgetFailure::Cancelled
                ))
            )
        })
    });
    let runtime_retirement = match &preparation_result {
        Ok(runtime) => runtime.retire().await,
        Err(_) => Ok(()),
    };
    if !first_poll_waiting
        || !reached
        || !matches!(
            selected_deadline_wait_pending,
            Some(std::task::Poll::Pending)
        )
        || !classified
        || !parser_cancel_cause
    {
        bail!(
            "selected preparation deadline lost precedence (prepare first poll pending: \
             {first_poll_waiting}; real parser checkpoint reached: {reached}; deadline branch \
             waited for worker: {}; typed deadline retained: {classified}; original parser \
             cancellation cause retained: {parser_cancel_cause})",
            matches!(
                selected_deadline_wait_pending,
                Some(std::task::Poll::Pending)
            ),
        );
    }
    runtime_retirement.context("deadline regression runtime cleanup failed")?;
    drop(workspace);
    Ok(())
}
