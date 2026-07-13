use maestria_domain::{DomainInput, KernelState, LogicalTick};

use crate::test_helpers;
use crate::{FeedbackError, MaestriaRuntime, RuntimeConfig};

#[test]
fn feedback_reports_capacity_without_waiting() -> Result<(), FeedbackError> {
    let config = RuntimeConfig {
        input_buffer_size: 1,
        ..RuntimeConfig::default()
    };
    let (runtime, input_rx) = MaestriaRuntime::new(
        config,
        KernelState::new(),
        test_helpers::test_adapters(),
        test_helpers::test_governance(),
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
