use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;
use std::time::Duration;

#[tokio::test]
async fn cancellation_drops_child_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    let adapter = adapter();
    let mut req = shell_request("cat", 60000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let fut = adapter.execute(req);

    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(fut);

    tokio::time::sleep(Duration::from_millis(100)).await;

    Ok(())
}
