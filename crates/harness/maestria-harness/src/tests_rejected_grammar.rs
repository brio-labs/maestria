use super::test_helpers::{adapter, shell_request};
use super::*;

#[tokio::test]
async fn rejects_unknown_program() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter().execute(shell_request("ls -la", 5000)).await;
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}

#[tokio::test]
async fn rejects_metacharacter_redirect() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter()
        .execute(shell_request("echo foo > bar", 5000))
        .await;
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput, got {result:?}"
    );
    Ok(())
}

#[tokio::test]
async fn rejects_metacharacter_pipe() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter()
        .execute(shell_request("echo foo | cat", 5000))
        .await;
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}

#[tokio::test]
async fn rejects_metacharacter_dollar() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter().execute(shell_request("echo $HOME", 5000)).await;
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}

#[tokio::test]
async fn rejects_metacharacter_backtick() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter()
        .execute(shell_request("echo `whoami`", 5000))
        .await;
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}

#[tokio::test]
async fn rejects_metacharacter_semicolon() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter()
        .execute(shell_request("echo a; echo b", 5000))
        .await;
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}

#[tokio::test]
async fn rejects_metacharacter_ampersand() -> Result<(), Box<dyn std::error::Error>> {
    let result = adapter()
        .execute(shell_request("echo a & echo b", 5000))
        .await;
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}
