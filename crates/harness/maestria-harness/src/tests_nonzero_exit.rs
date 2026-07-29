use super::test_helpers::{adapter, shell_request};
use maestria_ports::HarnessAdapter;
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

#[tokio::test]
async fn cat_continues_after_operand_error() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let valid = root.path().join("valid.txt");
    let missing = root.path().join("missing.txt");
    std::fs::write(&valid, b"after-error\n")?;

    let mut req = shell_request(
        &format!("cat {} {}", missing.display(), valid.display()),
        5000,
    );
    req.working_directory = root.path().to_path_buf();
    req.readable_roots = vec![root.path().to_path_buf()];
    let outcome = adapter().execute(req).await?;

    assert_eq!(outcome.exit_code, 1);
    assert_eq!(outcome.stdout, b"after-error\n");
    assert!(String::from_utf8_lossy(&outcome.stderr).contains("missing.txt"));
    Ok(())
}
