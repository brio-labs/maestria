use super::test_helpers::{adapter, shell_request};
use maestria_ports::{HarnessAdapter, PortError};
use std::path::PathBuf;

#[tokio::test]
async fn rejects_cat_path_before_any_output() -> Result<(), Box<dyn std::error::Error>> {
    let mut request = shell_request("cat /etc/hostname", 5000);
    request.readable_roots = vec![PathBuf::from("/tmp")];
    let authorization = crate::command::authorize_paths(&request)?;
    let result = crate::process::execute_command(
        "cat",
        &["/etc/hostname".to_string()],
        &request,
        &authorization,
    )
    .await;

    assert!(
        matches!(
            result,
            Err(PortError::InvalidInputContext {
                context: "cat path outside readable roots",
                ..
            })
        ),
        "expected contextual policy error, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_device_operand_without_hanging() -> Result<(), Box<dyn std::error::Error>> {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        adapter().execute(shell_request("cat /dev/zero", 5000)),
    )
    .await??;

    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("unsupported file type"),
        "expected bounded unsupported-file diagnostic, got {:?}",
        result.stderr
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn rejects_fifo_operand_without_hanging() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::ffi::OsStrExt;

    let root = tempfile::tempdir()?;
    let fifo = root.path().join("stream");
    let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes())?;
    // SAFETY: the path is NUL-free and points into the live temporary root.
    let result = unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) };
    assert_eq!(
        result,
        0,
        "mkfifo failed: {}",
        std::io::Error::last_os_error()
    );

    let mut request = shell_request("cat stream", 5000);
    request.working_directory = root.path().to_path_buf();
    request.readable_roots = vec![root.path().to_path_buf()];
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        adapter().execute(request),
    )
    .await??;

    assert_eq!(outcome.exit_code, 1);
    assert!(outcome.stdout.is_empty());
    assert!(String::from_utf8_lossy(&outcome.stderr).contains("unsupported file type"));
    Ok(())
}
