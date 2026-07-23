use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;

#[tokio::test]
async fn timeout_on_slow_command() -> Result<(), Box<dyn std::error::Error>> {
    let mut req = shell_request("cat /dev/urandom", 200);
    req.readable_roots = vec![PathBuf::from("/dev")];
    let result = adapter().execute(req).await;
    assert!(
        matches!(result, Err(PortError::Internal { .. })),
        "expected timeout Internal error, got {result:?}"
    );
    Ok(())
}
