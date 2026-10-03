use super::super::contract_tests::*;
use super::super::*;

/// Drive an immediately-ready future to completion without an async runtime.
///
/// The kernel crate cannot depend on Tokio (Rule 10), and the in-memory
/// adapters resolve their futures without waiting, so a no-op-waker poll loop
/// is sufficient to run the shared async contract suites.
fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    let waker = Waker::noop();
    let mut future = pin!(future);
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

#[test]
fn in_memory_harness_adapter_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    block_on(assert_harness_adapter_round_trip(
        &InMemoryHarnessAdapter::new(),
    ))?;
    Ok(())
}
