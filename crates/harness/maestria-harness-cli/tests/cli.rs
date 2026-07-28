use std::error::Error;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
#[test]
fn failing_allowed_command_propagates_exit_and_keeps_stderr_on_stderr() -> Result<(), Box<dyn Error>>
{
    let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let working_directory = std::env::temp_dir().join(format!(
        "maestria-harness-cli-{}-{suffix}",
        std::process::id()
    ));
    std::fs::create_dir(&working_directory)?;

    let output = Command::new(env!("CARGO_BIN_EXE_maestria-harness-cli"))
        .args(["--command", "cat missing-harness-cli-regression-input.txt"])
        .arg("--working-directory")
        .arg(&working_directory)
        .output();
    let cleanup = std::fs::remove_dir_all(&working_directory);
    let output = output?;
    cleanup?;

    assert!(
        !output.status.success(),
        "failing child unexpectedly succeeded"
    );
    assert_ne!(
        output.status.code(),
        Some(0),
        "failing child did not propagate a nonzero exit code"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("Exit code:"),
        "CLI metadata missing from stdout: {stdout}"
    );
    assert!(
        !stdout.contains("--- STDERR ---")
            && !stdout.contains("missing-harness-cli-regression-input.txt"),
        "child stderr leaked to stdout: {stdout}"
    );
    assert!(
        stderr.contains("--- STDERR ---"),
        "stderr label missing: {stderr}"
    );
    assert!(
        stderr.contains("missing-harness-cli-regression-input.txt"),
        "child stderr missing from stderr stream: {stderr}"
    );
    Ok(())
}
