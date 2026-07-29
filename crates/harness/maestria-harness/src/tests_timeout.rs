use super::test_helpers::{adapter, shell_request};
use maestria_ports::HarnessAdapter;
use std::path::PathBuf;

#[tokio::test]
async fn non_regular_device_is_rejected_without_waiting() -> Result<(), Box<dyn std::error::Error>>
{
    let mut req = shell_request("cat /dev/urandom", 200);
    req.readable_roots = vec![PathBuf::from("/tmp"), PathBuf::from("/dev")];
    let outcome = adapter().execute(req).await?;
    assert_eq!(outcome.exit_code, 1);
    assert!(
        String::from_utf8_lossy(&outcome.stderr).contains("unsupported file type"),
        "expected typed file-kind diagnostic, got {outcome:?}"
    );
    Ok(())
}
