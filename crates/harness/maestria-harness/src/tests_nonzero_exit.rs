use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;

#[tokio::test]
async fn cat_nonexistent_file_returns_nonzero() -> Result<(), Box<dyn std::error::Error>> {
    let mut req = shell_request("cat /tmp/maestria_nonexistent_xyz", 5000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let outcome = adapter().execute(req).await?;
    assert_ne!(
        outcome.exit_code, 0,
        "expected nonzero exit for missing file"
    );
    Ok(())
}
