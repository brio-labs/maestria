use crate::{MaestriaRuntime, RuntimeConfig};
use maestria_domain::{DomainInput, KernelState, LogicalTick};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn cancelled_reservation_does_not_consume_command_capacity()
-> Result<(), Box<dyn std::error::Error>> {
    let (runtime, _input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            input_buffer_size: 1,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        crate::test_helpers::test_adapters(),
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let held = handle.reserve_submission().await?;
    let blocked_handle = handle.clone();
    let blocked = tokio::spawn(async move { blocked_handle.reserve_submission().await });
    tokio::task::yield_now().await;
    assert!(!blocked.is_finished());
    blocked.abort();
    let blocked_result = blocked.await;
    assert!(blocked_result.is_err());

    drop(held);
    let replacement =
        tokio::time::timeout(Duration::from_secs(1), handle.reserve_submission()).await??;
    drop(replacement);
    Ok(())
}

#[test]
fn zero_input_buffer_size_is_normalized_before_channel_construction() {
    let (runtime, _input_rx) = MaestriaRuntime::new(
        RuntimeConfig {
            input_buffer_size: 0,
            ..RuntimeConfig::default()
        },
        KernelState::new(),
        crate::test_helpers::test_adapters(),
        crate::test_helpers::test_governance(),
    );
    assert_eq!(runtime.config.input_buffer_size, 1);
}

#[tokio::test]
async fn reserved_submission_is_accepted_before_waiting_for_result()
-> Result<(), Box<dyn std::error::Error>> {
    let (runtime, input_rx) = MaestriaRuntime::new(
        RuntimeConfig::default(),
        KernelState::new(),
        crate::test_helpers::test_adapters(),
        crate::test_helpers::test_governance(),
    );
    let handle = runtime.handle();
    let permit = handle.reserve_submission().await?;
    let submission = tokio::spawn(async move {
        permit
            .submit(DomainInput::ClockTick(LogicalTick::new(1)))
            .await
    });
    tokio::task::yield_now().await;
    assert!(!submission.is_finished());

    let shutdown = CancellationToken::new();
    let run_shutdown = shutdown.clone();
    let run = tokio::spawn(runtime.with_graceful_shutdown().run(input_rx, run_shutdown));
    let result = tokio::time::timeout(Duration::from_secs(2), submission).await???;
    assert_eq!(result.correlation_id, 1);
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(2), run).await???;
    Ok(())
}
