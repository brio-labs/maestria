use crate::test_support::*;
use maestria_domain::{
    ArtifactDetected, ArtifactId, FetchWebRequest, FetchWebRequested, LogicalTick, content_hash,
};
use maestria_ports::EventFilter;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn parse_artifact_no_deadlock_at_max_concurrency_one()
-> Result<(), Box<dyn std::error::Error>> {
    let event_log = Arc::new(InMemoryEventLog::new());
    let adapters = Adapters {
        event_log: event_log.clone(),
        ..crate::test_helpers::test_adapters()
    };
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            max_concurrent_effects: 1,
            default_effect_timeout: Duration::from_secs(5),
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

    let source_bytes = b"fn main() {}".to_vec();
    let source_hash = content_hash(&source_bytes);
    let artifact_id = ArtifactId::new(1);

    // Send ArtifactDetected input — the domain loop produces a
    // ParseArtifact effect, whose handler enqueues ParserStarted and
    // then runs the persistence barrier. With max_concurrent_effects=1,
    // the barrier must not deadlock waiting for the PersistEvent.
    input_tx
        .send(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title: "deadlock-test".to_string(),
            source_path: "/repo/deadlock.rs".to_string(),
            source_bytes,
            content_hash: source_hash,
        }))
        .await?;

    // Wait for the ParserStarted event to be persisted (proves no deadlock).
    let barrier_passed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let events = event_log
                .scan(EventFilter { artifact_id: None })
                .map_or(Vec::new(), |events| events);
            if events.iter().any(|e| {
                matches!(&e.event, DomainEvent::ParserStarted { artifact_id: id, .. } if *id == artifact_id)
            }) {
                break true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;

    shutdown.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(1), run).await;

    let no_deadlock = matches!(barrier_passed, Ok(true));
    assert!(
        no_deadlock,
        "ParserStarted persistence barrier must not deadlock at max_concurrent_effects=1"
    );
    Ok(())
}

/// Verify that a failing effect _not_ handled inline (PersistEvent) goes
/// through the spawned executor path and that the runtime is cancelled when
/// the effect fails after all retries are exhausted. This exercises the
/// async supervisor boundary that previously silently discarded
/// EffectFailure values from spawned tasks.
#[tokio::test]
async fn spawned_effect_failure_propagates_to_supervisor_and_cancels_runtime()
-> Result<(), Box<dyn std::error::Error>> {
    // An unseeded InMemoryWebFetcher returns NotFound for any URL that
    // hasn't been seeded, so the FetchWeb effect (non-PersistEvent,
    // always spawned) will fail.
    let adapters = crate::test_helpers::test_adapters();
    let governance = crate::test_helpers::test_governance();
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            scope: Scope::new(vec![], vec![], vec![], vec![], true),
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

    // Trigger FetchWeb effect via domain input — the domain handler
    // produces a recognised FetchWeb effect, which the executor schedules
    // as a spawned task (not PersistEvent, so the spawned path).
    input_tx
        .send(DomainInput::FetchWebRequested(FetchWebRequested {
            request: FetchWebRequest {
                url: "https://example.com/missing".to_string(),
                max_bytes: 1024,
                max_requests: 1,
                max_latency_ms: 1000,
                allowed_domains: vec![],
                allowed_content_types: vec![],
            },
        }))
        .await?;

    // The runtime should shut down because the spawned FetchWeb effect
    // fails (no seeded URL) and now propagates the failure.
    tokio::time::timeout(Duration::from_secs(2), run).await??;
    assert!(shutdown.is_cancelled());
    Ok(())
}

// ── feedback capacity ─────────────────────────────────────────────────

#[test]
fn feedback_reports_capacity_without_waiting() -> Result<(), FeedbackError> {
    let config = RuntimeConfig {
        input_buffer_size: 1,
        ..RuntimeConfig::default()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        config,
        KernelState::new(),
        crate::test_helpers::test_adapters(),
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    handle.try_send_feedback(DomainInput::ClockTick(LogicalTick::new(1)))?;
    assert_eq!(
        handle.try_send_feedback(DomainInput::ClockTick(LogicalTick::new(2))),
        Err(FeedbackError::CapacityFull)
    );
    drop(input_rx);
    assert_eq!(
        handle.try_send_feedback(DomainInput::ClockTick(LogicalTick::new(3))),
        Err(FeedbackError::RuntimeShutdown)
    );
    Ok(())
}
