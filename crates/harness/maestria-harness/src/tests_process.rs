use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;

#[tokio::test]
async fn reports_spawn_failures_with_context() -> Result<(), Box<dyn std::error::Error>> {
    let mut request = shell_request("cat /etc/hostname", 5000);
    request.working_directory = PathBuf::from("/definitely/missing/maestria-working-directory");

    let result = adapter().execute(request).await;

    assert!(
        matches!(
            result,
            Err(PortError::InternalContext {
                context: "spawn harness child process",
                ref source,
            }) if source.starts_with("cat: ")
        ),
        "expected contextual spawn error, got {result:?}"
    );
    Ok(())
}
