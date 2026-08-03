use super::test_helpers::{adapter, shell_request};
use maestria_ports::HarnessAdapter;
use std::future::Future;
use std::path::PathBuf;

#[tokio::test]
async fn cancellation_drops_in_flight_execution_cleanly() -> Result<(), Box<dyn std::error::Error>>
{
    let adapter = adapter();
    let mut req = shell_request("cat", 60000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let mut fut = Box::pin(adapter.execute(req));

    // Drive the execution future at least once so the drop path runs with
    // in-flight state, then drop it before completion: dropping must not
    // hang or leave the adapter unusable.
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    let _ = fut.as_mut().poll(&mut context);
    drop(fut);

    let outcome = adapter.execute(shell_request("echo", 60000)).await?;
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&outcome.stdout), "\n");
    Ok(())
}
