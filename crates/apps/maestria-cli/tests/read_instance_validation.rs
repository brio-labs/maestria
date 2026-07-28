use std::fs;

mod common;

use common::{TempDir, assert_init_ok, assert_ok_lines, run};

fn assert_err(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(args)?;
    assert_ne!(
        code, 0,
        "command unexpectedly succeeded: {:?}\nstdout: {stdout}",
        args
    );
    assert!(
        stdout.trim().is_empty(),
        "failed command wrote unexpected stdout: {stdout}"
    );
    Ok(stderr)
}

fn assert_task_start(
    instance_path: &str,
    title: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let args: Vec<&str> = vec!["task", "start", "-i", instance_path, title];
    let stdout = assert_ok_lines(&args, 1)?;
    let line = stdout.trim();
    let task_prefix = "task=";
    let task_start = line
        .find(task_prefix)
        .ok_or("task start output missing task=")?;
    let after_task = &line[task_start + task_prefix.len()..];
    let task_id: String = after_task
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    assert!(
        !task_id.is_empty(),
        "could not extract task id from: {line}"
    );
    Ok(task_id)
}

#[test]
fn query_commands_require_an_initialized_instance() -> Result<(), Box<dyn std::error::Error>> {
    let instance = TempDir::new("maestria-test-uninitialized-instance")?;
    let error = assert_err(&[
        "search",
        "-i",
        &instance.path().to_string_lossy(),
        "anything",
    ])?;
    assert!(
        error.contains("instance manifest is missing"),
        "unexpected uninitialized-instance error: {error}"
    );
    let error = assert_err(&[
        "open-evidence",
        "-i",
        &instance.path().to_string_lossy(),
        "--evidence-id",
        "1",
    ])?;
    assert!(
        error.contains("instance manifest is missing"),
        "unexpected uninitialized-instance evidence error: {error}"
    );
    Ok(())
}

#[test]
fn read_commands_reject_existing_database_without_valid_manifest()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-read-manifest-workspace")?;
    let instance = TempDir::new("maestria-test-read-manifest-instance")?;
    let ip = instance.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), workspace.path().to_string_lossy().as_ref())?;
    let _task_id = assert_task_start(ip.as_ref(), "Persisted before manifest removal")?;
    common::write_file(
        workspace.path(),
        "read-validation.md",
        "persisted state must still require a valid manifest",
    )?;
    common::assert_index_ok(
        ip.as_ref(),
        &workspace
            .path()
            .join("read-validation.md")
            .to_string_lossy(),
    )?;

    let manifest = instance.path().join("manifest.txt");
    fs::remove_file(&manifest)?;

    for args in [
        ["memory", "candidates", "-i", ip.as_ref()],
        ["task", "show", "-i", ip.as_ref()],
    ] {
        let error = assert_err(&args)?;
        assert!(
            error.contains("instance manifest is missing"),
            "read command unexpectedly bypassed missing manifest: {error}"
        );
    }

    fs::write(&manifest, "not a valid instance manifest\n")?;
    for args in [
        ["memory", "candidates", "-i", ip.as_ref()],
        ["task", "show", "-i", ip.as_ref()],
    ] {
        let error = assert_err(&args)?;
        assert!(
            error.contains("parse instance manifest"),
            "read command unexpectedly bypassed invalid manifest: {error}"
        );
    }

    Ok(())
}
