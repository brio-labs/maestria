use super::test_helpers::{adapter, shell_request};
use super::*;
use std::path::PathBuf;

#[tokio::test]
async fn echo_returns_stdout() -> Result<(), Box<dyn std::error::Error>> {
    let outcome = adapter()
        .execute(shell_request("echo hello world", 5000))
        .await?;
    assert_eq!(outcome.exit_code, 0);
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    assert!(stdout.contains("hello world"), "stdout: {stdout:?}");
    Ok(())
}

#[tokio::test]
async fn pwd_returns_working_directory() -> Result<(), Box<dyn std::error::Error>> {
    let mut req = shell_request("pwd", 5000);
    req.working_directory = PathBuf::from("/tmp");
    let outcome = adapter().execute(req).await?;
    assert_eq!(outcome.exit_code, 0);
    let stdout = String::from_utf8_lossy(&outcome.stdout);
    assert!(stdout.contains("/tmp"), "stdout: {stdout:?}");
    Ok(())
}

#[tokio::test]
async fn cat_reads_file_in_readable_root() -> Result<(), Box<dyn std::error::Error>> {
    let path = "/tmp/maestria_harness_cat_test.txt";
    std::fs::write(path, b"meow\n")?;

    let mut req = shell_request(&format!("cat {path}"), 5000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let outcome = adapter().execute(req).await?;
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.stdout, b"meow\n");

    std::fs::remove_file(path).ok();
    Ok(())
}
