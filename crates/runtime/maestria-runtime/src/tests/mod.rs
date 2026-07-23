//! Consolidated integration tests for `maestria-runtime`.
//!
//! Each submodule owns one behavior family, following the
//! "one concept per module" rule.

use maestria_domain::{
    CompleteTaskInput, DomainEvent, DomainEventEnvelope, DomainInput, KernelState, TaskId,
    ValidationReportId,
};
use maestria_ports::{EventFilter, EventLog, InMemoryEventLog, PortError};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

mod barrier;
mod blob;
mod card_index;
mod completion;
mod evidence;
mod graph;
mod harness;
mod parse;
mod pdf;
mod persist;
mod resume;
mod search;
mod search_validation;
mod shell_policy;
mod supervision;

/// EventLog implementation that always fails, useful for testing
/// runtime shutdown on persistence errors.
#[derive(Default)]
pub struct FailingEventLog;

impl EventLog for FailingEventLog {
    fn append(&self, _event: DomainEventEnvelope) -> Result<(), PortError> {
        Err(PortError::Downstream {
            message: "event store unavailable".to_string(),
        })
    }

    fn scan(&self, _filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        Ok(Vec::new())
    }
}

/// Shared helper for task-completion integration tests.
///
/// Seeds the event log with `seed_events`, spins up a runtime, sends
/// `CompleteTask`, waits for a deterministic barrier (`ClockTick`), then
/// returns any `TaskCompletionRecorded` events.
pub async fn run_complete_task_test(
    mut state: KernelState,
    governance: crate::Governance,
    task_id: TaskId,
    report_id: ValidationReportId,
    seed_events: Vec<DomainEvent>,
) -> Result<Vec<DomainEventEnvelope>, Box<dyn std::error::Error>> {
    let event_log = Arc::new(InMemoryEventLog::new());
    for event in seed_events {
        let envelope = DomainEventEnvelope {
            id: maestria_domain::EventId::new(state.event_log.len() as u64 + 1),
            sequence: maestria_domain::SequenceNumber::new(state.event_log.len() as u64 + 1),
            event,
        };
        event_log
            .append(envelope.clone())
            .map_err(|e| format!("seed append failed: {:?}", e))?;
        state.event_log.push(envelope);
    }
    let adapters = crate::Adapters {
        event_log: event_log.clone(),
        ..crate::test_helpers::test_adapters()
    };

    let (runtime, input_rx) =
        crate::MaestriaRuntime::new(crate::RuntimeConfig::default(), state, adapters, governance);

    let input_tx = runtime.handle().input_tx;
    let shutdown_token = CancellationToken::new();
    let runtime_handle = tokio::spawn(runtime.run(input_rx, shutdown_token.clone()));

    input_tx
        .send(DomainInput::CompleteTask(CompleteTaskInput {
            task_id,
            validation_report_id: report_id,
        }))
        .await
        .map_err(|e| format!("send failed: {}", e))?;

    let sync_tick = maestria_domain::LogicalTick::new(999);
    input_tx
        .send(DomainInput::ClockTick(sync_tick))
        .await
        .map_err(|e| format!("send tick failed: {}", e))?;

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let all_events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_err(|e| format!("scan failed: {:?}", e))?;
            if all_events
                .iter()
                .any(|e| matches!(e.event, DomainEvent::TickObserved { at } if at == sync_tick))
            {
                return Ok::<(), Box<dyn std::error::Error>>(());
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .map_err(|_| "timeout waiting for deterministic barrier (ClockTick)")??;

    shutdown_token.cancel();
    let _ = runtime_handle.await;

    let all_events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_err(|e| format!("scan failed: {:?}", e))?;
    let new_events = all_events
        .into_iter()
        .filter(|e| matches!(e.event, DomainEvent::TaskCompletionRecorded { .. }))
        .collect();
    Ok(new_events)
}
