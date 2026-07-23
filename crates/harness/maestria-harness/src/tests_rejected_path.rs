use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;

#[tokio::test]
async fn cat_rejects_path_outside_readable_roots() -> Result<(), Box<dyn std::error::Error>> {
    let mut req = shell_request("cat /etc/hostname", 5000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let result = adapter().execute(req).await;
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for path outside roots, got {result:?}"
    );
    Ok(())
}
