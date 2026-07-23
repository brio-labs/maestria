use super::*;
use crate::command::filename_matches;
use maestria_ports::contract_tests::assert_harness_adapter_round_trip;
use std::path::PathBuf;
use std::time::Duration;

fn adapter() -> LocalShellHarnessAdapter {
    LocalShellHarnessAdapter
}

fn shell_request(command: &str, budget_ms: u64) -> HarnessRequest {
    HarnessRequest {
        run_id: maestria_ports::HarnessRunId::new(1),
        command: command.to_string(),
        working_directory: PathBuf::from("/tmp"),
        duration_budget: Duration::from_millis(budget_ms),
        class: HarnessCommandClass::Shell,
        readable_roots: vec![
            PathBuf::from("/"),
            PathBuf::from("/tmp"),
            PathBuf::from("/dev"),
        ],
        blocked_paths: vec![],
        blocked_patterns: vec![],
    }
}

// ── success cases ──────────────────────────────────────────────────

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
    // Write a temporary file in /tmp, then cat it.
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

// ── nonzero exit ───────────────────────────────────────────────────

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

// ── rejected grammar ───────────────────────────────────────────────

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

// ── rejected path ──────────────────────────────────────────────────

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

// ── timeout ────────────────────────────────────────────────────────

#[tokio::test]
async fn timeout_on_slow_command() -> Result<(), Box<dyn std::error::Error>> {
    // cat /dev/urandom produces endless output → pipe fills → process blocks.
    let mut req = shell_request("cat /dev/urandom", 200);
    req.readable_roots = vec![PathBuf::from("/dev")];
    let result = adapter().execute(req).await;
    assert!(
        matches!(result, Err(PortError::Internal { .. })),
        "expected timeout Internal error, got {result:?}"
    );
    Ok(())
}

// ── cancellation safety (drop test) ────────────────────────────────

#[tokio::test]
async fn cancellation_drops_child_cleanly() -> Result<(), Box<dyn std::error::Error>> {
    // Spawn a long-running cat that blocks on stdin.
    // Drop the future before it completes — kill_on_drop should reap the child.
    let adapter = adapter();
    let mut req = shell_request("cat", 60000);
    req.readable_roots = vec![PathBuf::from("/tmp")];
    let fut = adapter.execute(req);

    // Give the child time to start, then cancel.
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(fut);

    // Give the OS time to reap.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // If we got here without zombie processes or panics, the test passes.
    // (A zombie would manifest as a leaked child that the test runtime
    //  would only catch later — this is a best-effort smoke test.)
    Ok(())
}

// ── capabilities contract ──────────────────────────────────────────

#[tokio::test]
async fn capabilities_report_shell_only() -> Result<(), Box<dyn std::error::Error>> {
    let caps = adapter().capabilities()?;
    assert!(caps.read_enabled);
    assert!(!caps.write_enabled);
    assert!(!caps.web_enabled);
    assert_eq!(caps.command_classes, vec![HarnessCommandClass::Shell]);
    Ok(())
}
// ── filename pattern matching tests ────────────────────────────

#[test]
fn filename_matches_exact() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches(".env", ".env"));
    assert!(!filename_matches(".env", "other"));
    Ok(())
}

#[test]
fn filename_matches_wildcard_suffix() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches("secret.key", "*.key"));
    assert!(!filename_matches("key.txt", "*.key"));
    Ok(())
}

#[test]
fn filename_matches_wildcard_prefix() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches(".env.prod", ".env.*"));
    assert!(!filename_matches(".env", ".env.*"));
    Ok(())
}

#[test]
fn filename_matches_question_wildcard() -> Result<(), Box<dyn std::error::Error>> {
    assert!(filename_matches("a.key", "?.key"));
    assert!(!filename_matches("ab.key", "?.key"));
    Ok(())
}

#[tokio::test]
async fn cat_rejects_blocked_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let keyfile = tmp.path().join("secret.key");
    std::fs::write(&keyfile, b"keydata")?;
    let mut req = shell_request(&format!("cat {}", keyfile.display()), 5000);
    req.readable_roots = vec![tmp.path().to_path_buf()];
    req.blocked_patterns = vec!["*.key".into()];
    assert!(adapter().execute(req).await.is_err());
    Ok(())
}

#[tokio::test]
async fn cat_rejects_dotenv_pattern() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let envfile = tmp.path().join(".env");
    std::fs::write(&envfile, b"SECRET=xyz")?;
    let mut req = shell_request(&format!("cat {}", envfile.display()), 5000);
    req.readable_roots = vec![tmp.path().to_path_buf()];
    req.blocked_patterns = vec![".env".into()];
    assert!(adapter().execute(req).await.is_err());
    Ok(())
}

// ── shared contract suite (Rule 25) ────────────────────────────────

#[tokio::test]
async fn harness_adapter_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_harness_adapter_round_trip(&adapter()).await?;
    Ok(())
}
