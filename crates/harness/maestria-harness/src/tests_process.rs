use super::test_helpers::{adapter, shell_request};
use maestria_ports::{HarnessAdapter, PortError};
use std::path::PathBuf;

#[tokio::test]
async fn reports_spawn_failures_with_context() -> Result<(), Box<dyn std::error::Error>> {
    let mut request = shell_request("cat /etc/hostname", 5000);
    request.working_directory = PathBuf::from("/definitely/missing/maestria-working-directory");
    let result = crate::process::spawn_and_collect("cat", &[], &request).await;

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

#[tokio::test]
async fn rejects_output_overflow_without_hanging() -> Result<(), Box<dyn std::error::Error>> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        adapter().execute(shell_request("cat /dev/zero", 5000)),
    )
    .await?;

    assert!(
        matches!(
            result,
            Err(PortError::InternalContext {
                context: "harness process output limit exceeded",
                ref source,
            }) if source.contains("stdout") || source.contains("stderr")
        ),
        "expected typed output-limit error, got {result:?}"
    );
    Ok(())
}
