use maestria_cli::test_support::*;
use std::path::Path;

fn assert_index_and_get_evidence(
    instance_path: &str,
    workspace: &Path,
    filename: &str,
    content: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    write_file(workspace, filename, content)?;
    let file = workspace.join(filename).to_string_lossy().into_owned();
    assert_index_ok(instance_path, &file)?;
    let body = match content
        .lines()
        .find(|line| !line.starts_with('#') && !line.trim().is_empty())
    {
        Some(body) => body,
        None => content,
    };
    let query_word = body
        .split_whitespace()
        .next()
        .ok_or("content must have at least one word")?;
    let (_chunk_id, evidence_id) = assert_search_finds(instance_path, query_word)?;
    Ok(evidence_id)
}
#[test]
fn memory_propose_and_list_survives_restart() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-workspace")?;
    let instance = TempDir::new("maestria-test-mem-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let evidence_id = assert_index_and_get_evidence(
        ip.as_ref(),
        workspace.path(),
        "notes.md",
        "# Architecture\n\nThe project uses Rust for performance.\n",
    )?;
    let (code, stdout, stderr) = run(&[
        "memory",
        "propose",
        "--text",
        "The project uses Rust",
        "--evidence-id",
        &evidence_id,
        "--confidence-milli",
        "750",
        "-i",
        ip.as_ref(),
    ])?;
    eprintln!("PROPOSE code={code} stdout={stdout:?} stderr={stderr:?}");
    assert_eq!(
        code, 0,
        "propose failed: stdout={stdout:?} stderr={stderr:?}"
    );
    assert!(
        stdout.contains("proposed"),
        "propose output missing 'proposed': {stdout}"
    );
    let list_stdout = assert_ok_lines(&["memory", "candidates", "-i", ip.as_ref()], 1)?;
    assert!(
        list_stdout.contains("candidate="),
        "candidates list output missing candidate: {list_stdout}"
    );
    let list2_stdout = assert_ok_lines(&["memory", "candidates", "-i", ip.as_ref()], 1)?;
    assert_eq!(
        list_stdout, list2_stdout,
        "candidate list must be identical after restart"
    );
    Ok(())
}
#[test]
fn memory_propose_empty_text_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-empty-text-workspace")?;
    let instance = TempDir::new("maestria-test-mem-empty-text-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let evidence_id = assert_index_and_get_evidence(
        ip.as_ref(),
        workspace.path(),
        "notes.md",
        "# Notes\n\nSome content here.\n",
    )?;
    let err = assert_err(&[
        "memory",
        "propose",
        "--text",
        "   ",
        "--evidence-id",
        &evidence_id,
        "--confidence-milli",
        "500",
        "-i",
        ip.as_ref(),
    ])?;
    assert!(
        err.contains("claim text must not be empty"),
        "expected empty text rejection, got: {err}"
    );
    Ok(())
}
#[test]
fn memory_propose_missing_evidence_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-missing-ev-workspace")?;
    let instance = TempDir::new("maestria-test-mem-missing-ev-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let err = assert_err(&[
        "memory",
        "propose",
        "--text",
        "Valid claim text",
        "--evidence-id",
        "999",
        "--confidence-milli",
        "500",
        "-i",
        ip.as_ref(),
    ])?;
    assert!(
        err.contains("evidence") && err.contains("not found"),
        "expected missing evidence rejection, got: {err}"
    );
    Ok(())
}
#[test]
fn memory_propose_no_evidence_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-no-ev-workspace")?;
    let instance = TempDir::new("maestria-test-mem-no-ev-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let err = assert_err(&[
        "memory",
        "propose",
        "--text",
        "Valid claim text",
        "--confidence-milli",
        "500",
        "-i",
        ip.as_ref(),
    ])?;
    assert!(
        err.contains("evidence") || err.contains("require"),
        "expected no-evidence rejection, got: {err}"
    );
    Ok(())
}
#[test]
fn memory_propose_invalid_confidence_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-conf-workspace")?;
    let instance = TempDir::new("maestria-test-mem-conf-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let evidence_id = assert_index_and_get_evidence(
        ip.as_ref(),
        workspace.path(),
        "notes.md",
        "# Notes\n\nSome content.\n",
    )?;
    let (code, stdout, stderr) = run(&[
        "memory",
        "propose",
        "--text",
        "Valid claim",
        "--evidence-id",
        &evidence_id,
        "--confidence-milli",
        "1001",
        "-i",
        ip.as_ref(),
    ])?;
    assert_ne!(code, 0, "confidence 1001 must be rejected by clap");
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("1000") || combined.contains("confidence"),
        "expected confidence range rejection, got: {combined}"
    );
    Ok(())
}
#[test]
fn memory_propose_does_not_promote() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-mem-no-promote-workspace")?;
    let instance = TempDir::new("maestria-test-mem-no-promote-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    let evidence_id = assert_index_and_get_evidence(
        ip.as_ref(),
        workspace.path(),
        "notes.md",
        "# Architecture\n\nThe project uses Rust for performance.\n",
    )?;
    assert_ok_lines(
        &[
            "memory",
            "propose",
            "--text",
            "The project uses Rust",
            "--evidence-id",
            &evidence_id,
            "--confidence-milli",
            "750",
            "-i",
            ip.as_ref(),
        ],
        1,
    )?;
    let list_stdout = assert_ok_lines(&["memory", "candidates", "-i", ip.as_ref()], 1)?;
    assert!(
        list_stdout.contains("candidate="),
        "candidate must be listed; promotion must not have occurred: {list_stdout}"
    );
    Ok(())
}
