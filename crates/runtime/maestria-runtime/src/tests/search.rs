use crate::test_support::*;
use crate::tests::FailingEventLog;
use maestria_domain::{DomainEvent, EvidenceId, LogicalTick, SearchExecutedInput};
use maestria_ports::{EventFilter, InMemoryEventLog};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

// ── SearchExecuted audit event lifecycle ──────────────────────────

#[tokio::test]
async fn search_executed_persists_and_is_observable() -> Result<(), Box<dyn std::error::Error>> {
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 2,
            default_effect_timeout: Duration::from_secs(2),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        governance,
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    input_tx
        .send(DomainInput::SearchExecuted(SearchExecutedInput {
            query: "audit test".to_string(),
            limit: 5,
            evidence_ids: vec![EvidenceId::new(10), EvidenceId::new(20)],
            pack_metadata: None,
            at: LogicalTick::new(3),
        }))
        .await?;

    // Wait for the event to appear in the event log.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_or(Vec::new(), |events| events);
            if events
                .iter()
                .any(|env| matches!(env.event, DomainEvent::SearchExecuted { .. }))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;

    let events = event_log
        .scan(EventFilter { artifact_id: None })
        .map_or(Vec::new(), |events| events);
    assert_eq!(events.len(), 1);
    match &events[0].event {
        DomainEvent::SearchExecuted {
            query,
            limit,
            evidence_ids,
            pack_metadata,
            at,
        } => {
            assert_eq!(query, "audit test");
            assert_eq!(*limit, 5);
            assert_eq!(
                evidence_ids,
                &vec![EvidenceId::new(10), EvidenceId::new(20)]
            );
            assert!(pack_metadata.is_none());
            assert_eq!(*at, LogicalTick::new(3));
        }
        _ => return Err("expected SearchExecuted event".to_string().into()),
    }

    shutdown.cancel();
    run.await?;
    Ok(())
}

#[tokio::test]
async fn search_executed_with_failing_event_log_stops_runtime()
-> Result<(), Box<dyn std::error::Error>> {
    let adapters = Adapters {
        event_log: Arc::new(FailingEventLog),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            default_effect_timeout: Duration::from_secs(1),
            max_retries: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        adapters,
        governance,
    );
    let input_tx = runtime.handle().input_tx;
    let shutdown = CancellationToken::new();
    let run = tokio::spawn(runtime.run(input_rx, shutdown.clone()));

    input_tx
        .send(DomainInput::SearchExecuted(SearchExecutedInput {
            query: "should fail".to_string(),
            limit: 1,
            evidence_ids: vec![],
            pack_metadata: None,
            at: LogicalTick::new(1),
        }))
        .await?;

    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}
