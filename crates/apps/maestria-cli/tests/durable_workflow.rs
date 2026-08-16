use maestria_cli::test_support::*;
use std::{fs, path::Path, time::Duration};
fn write_file_bytes(
    parent: &Path,
    name: &str,
    contents: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
    let path = parent.join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, contents)?;
    Ok(())
}
fn create_minimal_pdf(text: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let text_str = std::str::from_utf8(text)?;
    let mut pdf_text = String::with_capacity(text_str.len() + 8);
    for ch in text_str.chars() {
        match ch {
            '(' => pdf_text.push_str("\\("),
            ')' => pdf_text.push_str("\\)"),
            '\\' => pdf_text.push_str("\\\\"),
            _ => pdf_text.push(ch),
        }
    }
    let content_data = format!("BT\n/F1 12 Tf\n72 700 Td\n({pdf_text}) Tj\nET");
    let content_len = content_data.len();
    let font_obj = b"1 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n";
    let content_header = format!("2 0 obj\n<< /Length {content_len} >>\nstream\n");
    let content_footer = "\nendstream\nendobj\n";
    let page_obj = b"3 0 obj\n<< /Type /Page /Parent 4 0 R /MediaBox [0 0 612 792] \
/Contents 2 0 R /Resources << /Font << /F1 1 0 R >> >> >>\nendobj\n";
    let pages_obj = b"4 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
    let catalog_obj = b"5 0 obj\n<< /Type /Catalog /Pages 4 0 R >>\nendobj\n";
    let mut buf = Vec::with_capacity(1024);
    buf.extend_from_slice(b"%PDF-1.4\n");
    let off_font = buf.len();
    buf.extend_from_slice(font_obj);
    let off_content = buf.len();
    buf.extend_from_slice(content_header.as_bytes());
    buf.extend_from_slice(content_data.as_bytes());
    buf.extend_from_slice(content_footer.as_bytes());
    let off_page = buf.len();
    buf.extend_from_slice(page_obj);
    let off_pages = buf.len();
    buf.extend_from_slice(pages_obj);
    let off_catalog = buf.len();
    buf.extend_from_slice(catalog_obj);
    let xref_offset = buf.len();
    buf.extend_from_slice(b"xref\n0 6\n");
    buf.extend_from_slice(b"0000000000 65535 f\r\n");
    buf.extend_from_slice(format!("{off_font:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_content:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_page:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_pages:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_catalog:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 5 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );
    Ok(buf)
}
fn create_no_text_pdf() -> Vec<u8> {
    let page_obj = b"1 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";
    let pages_obj = b"2 0 obj\n<< /Type /Pages /Kids [1 0 R] /Count 1 >>\nendobj\n";
    let catalog_obj = b"3 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n";
    let mut buf = Vec::with_capacity(512);
    buf.extend_from_slice(b"%PDF-1.4\n");
    let off_page = buf.len();
    buf.extend_from_slice(page_obj);
    let off_pages = buf.len();
    buf.extend_from_slice(pages_obj);
    let off_catalog = buf.len();
    buf.extend_from_slice(catalog_obj);
    let xref_offset = buf.len();
    buf.extend_from_slice(b"xref\n0 4\n");
    buf.extend_from_slice(b"0000000000 65535 f\r\n");
    buf.extend_from_slice(format!("{off_page:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_pages:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_catalog:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 3 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );
    buf
}
fn assert_reindex_unchanged(
    instance_path: &str,
    file: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok(&["index", "-i", instance_path, file])?;
    assert!(
        stdout.contains("unchanged "),
        "expected 'unchanged' in re-index output: {stdout}"
    );
    assert!(
        stdout.contains("duration="),
        "expected run metrics in re-index output: {stdout}"
    );
    Ok(())
}
fn assert_open_evidence_ok(
    instance_path: &str,
    evidence_id_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(
        &[
            "open-evidence",
            "-i",
            instance_path,
            "--evidence-id",
            evidence_id_str,
        ],
        3,
    )?;
    let evidence_line = stdout
        .lines()
        .find(|line| line.starts_with("evidence="))
        .ok_or("evidence line not found")?;
    assert!(
        evidence_line.contains(evidence_id_str),
        "open-evidence should echo evidence id {evidence_id_str}: {stdout}"
    );
    assert!(
        stdout.contains("source=file"),
        "open-evidence missing source=file: {stdout}"
    );
    assert!(
        stdout.contains("notes.md"),
        "open-evidence missing source path: {stdout}"
    );
    assert!(
        stdout.contains("excerpt="),
        "open-evidence missing excerpt: {stdout}"
    );
    let excerpt_line = stdout
        .lines()
        .find(|line| line.starts_with("excerpt="))
        .ok_or("excerpt line not found")?;
    assert!(
        !excerpt_line["excerpt=".len()..].is_empty(),
        "excerpt is empty: {excerpt_line}"
    );
    assert!(
        stdout.contains("hash="),
        "open-evidence missing hash: {stdout}"
    );
    Ok(())
}
/// Out-of-scope sources are skipped, not fatal: the batch must succeed with
/// the file counted as `excluded` and never indexed.
fn assert_skip_outside(instance_path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok(&["index", "-i", instance_path, file])?;
    assert!(
        stdout.contains("policy skipped 1 sources (excluded)"),
        "expected excluded-skip for out-of-scope file, got: {stdout}"
    );
    Ok(())
}
/// Privacy-excluded direct targets are rejected at collection time: the
/// command fails with no stdout.
fn assert_reject_env(instance_path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let err = assert_err(&["index", "-i", instance_path, file])?;
    assert!(
        err.contains("excluded by privacy policy")
            || err.contains("outside the instance read scope"),
        "expected exclusion rejection for .env, got: {err}"
    );
    Ok(())
}
#[test]
fn durable_cli_workflow() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-workspace")?;
    let instance = TempDir::new("maestria-test-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    write_file(
        workspace.path(),
        "notes.md",
        "# Design Notes\n\nThe system uses a distributed ledger for consensus.\n",
    )?;
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes)?;
    assert_reindex_unchanged(ip.as_ref(), &notes)?;
    let events_before_search = status_event_count(ip.as_ref())?;
    let (_chunk_id, evidence_id) = assert_search_finds(ip.as_ref(), "distributed")?;
    assert_eq!(
        status_event_count(ip.as_ref())?,
        events_before_search + 1,
        "search must append exactly one audit event before output",
    );
    assert_open_evidence_ok(ip.as_ref(), &evidence_id)?;
    let outside = TempDir::new("maestria-test-outside")?;
    write_file(outside.path(), "sneaky.md", "# sneaky\n")?;
    let sneaky = outside
        .path()
        .join("sneaky.md")
        .to_string_lossy()
        .into_owned();
    assert_skip_outside(ip.as_ref(), &sneaky)?;
    write_file(workspace.path(), ".env", "SECRET=do_not_index")?;
    let env_file = workspace.path().join(".env").to_string_lossy().into_owned();
    assert_reject_env(ip.as_ref(), &env_file)?;
    Ok(())
}
#[test]
fn recursive_index_skips_default_privacy_paths() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-recursive-workspace")?;
    let instance = TempDir::new("maestria-test-recursive-instance")?;
    assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    )?;
    write_file(workspace.path(), "notes.md", "# Public note\n")?;
    write_file(
        workspace.path(),
        "credentials/leaked.md",
        "# Sensitive note\n",
    )?;
    let stdout = assert_ok(&[
        "index",
        "-i",
        &instance.path().to_string_lossy(),
        &workspace.path().to_string_lossy(),
        "--recursive",
    ])?;
    assert!(
        stdout.contains("notes.md"),
        "public note was not indexed: {stdout}"
    );
    assert!(
        !stdout.contains("leaked.md"),
        "privacy-excluded file was indexed: {stdout}"
    );
    Ok(())
}

#[test]
fn pdf_indexing_workflow() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-pdf-workspace")?;
    let instance = TempDir::new("maestria-test-pdf-instance")?;
    let stdout = assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    )?;
    assert!(
        stdout.contains("initialized"),
        "init stdout missing 'initialized': {stdout}"
    );
    let pdf_bytes = create_minimal_pdf(b"The system uses a distributed ledger for consensus.")?;
    write_file_bytes(workspace.path(), "paper.pdf", &pdf_bytes)?;
    let stdout = assert_ok(&[
        "index",
        "-i",
        &instance.path().to_string_lossy(),
        &workspace.path().join("paper.pdf").to_string_lossy(),
    ])?;
    assert!(
        stdout.contains("indexed "),
        "expected 'indexed' in index output: {stdout}"
    );
    let stdout = assert_ok(&[
        "search",
        "-i",
        &instance.path().to_string_lossy(),
        "distributed",
    ])?;
    let evidence_output_line = stdout
        .lines()
        .find(|line| line.contains("evidence="))
        .ok_or("search output missing evidence line")?;
    let kv = parse_kv(evidence_output_line);
    let evidence_id_str = kv
        .iter()
        .find(|(key, _)| *key == "evidence")
        .map(|(_, value)| *value)
        .ok_or("search output missing evidence=<id>")?;
    let stdout = assert_ok_lines(
        &[
            "open-evidence",
            "-i",
            &instance.path().to_string_lossy(),
            "--evidence-id",
            evidence_id_str,
        ],
        3,
    )?;
    assert!(
        stdout.contains("source=pdf"),
        "open-evidence missing source=pdf: {stdout}"
    );
    assert!(
        stdout.contains("pages=1-1"),
        "open-evidence missing pages=1-1: {stdout}"
    );
    assert!(
        stdout.contains("excerpt="),
        "open-evidence missing excerpt: {stdout}"
    );
    let excerpt_line = stdout
        .lines()
        .find(|line| line.starts_with("excerpt="))
        .ok_or("excerpt line not found")?;
    assert!(
        !excerpt_line["excerpt=".len()..].is_empty(),
        "excerpt is empty: {excerpt_line}"
    );
    Ok(())
}
#[test]
fn pdf_no_text_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-pdf-empty-workspace")?;
    let instance = TempDir::new("maestria-test-pdf-empty-instance")?;
    assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    )?;
    let empty_pdf = create_no_text_pdf();
    write_file_bytes(workspace.path(), "scanned.pdf", &empty_pdf)?;
    // A no-text PDF never reaches `IndexStatus::Indexed`, so the CLI burns its
    // own 30s index budget before reporting a timeout. The batch must fail
    // (non-zero exit) but still print the summary line to stdout; the failure
    // detail goes to stderr. Run with a larger harness budget than the
    // default 30s cap so the product's timeout, not the harness kill,
    // produces the expected error (race-free).
    let (code, stdout, stderr) = run_bounded(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().join("scanned.pdf").to_string_lossy(),
        ],
        Duration::from_secs(60),
    )?;
    assert_ne!(
        code, 0,
        "command unexpectedly succeeded: {stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("failed 1"),
        "expected failed count in summary, got: {stdout}"
    );
    assert!(
        stderr.contains("failed artifact"),
        "expected per-artifact failure on stderr, got: {stderr}"
    );
    let stdout = assert_ok(&[
        "search",
        "-i",
        &instance.path().to_string_lossy(),
        "anything",
    ])?;
    assert!(
        stdout.contains("search_status=NoEvidenceFound"),
        "expected explicit no-evidence status, got: {stdout}"
    );
    Ok(())
}
fn assert_task_show(
    instance_path: &str,
    task_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["task", "show", "-i", instance_path, task_id], 1)?;
    Ok(stdout.trim().to_string())
}
#[test]
fn task_add_evidence_and_show() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-workspace")?;
    let instance = TempDir::new("maestria-test-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    write_file(
        workspace.path(),
        "notes.md",
        "# Design Notes\n\nThe system uses a distributed ledger for consensus.\n",
    )?;
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes)?;
    let (_chunk_id, evidence_id) = assert_search_finds(ip.as_ref(), "distributed")?;
    let task_id = assert_task_start(ip.as_ref(), "Review")?;
    assert!(!task_id.is_empty(), "task id must not be empty");
    let stdout = assert_ok(&[
        "task",
        "add-evidence",
        "-i",
        ip.as_ref(),
        &task_id,
        "--evidence-id",
        &evidence_id,
    ])?;
    assert!(
        stdout.contains("linked evidence="),
        "add-evidence output missing confirmation: {stdout}"
    );
    let task_line = assert_task_show(ip.as_ref(), &task_id)?;
    let expected = format!("EvidenceId({evidence_id})");
    assert!(
        task_line.contains(&expected),
        "task show must list linked evidence {evidence_id}: {task_line}"
    );
    Ok(())
}
#[test]
fn task_request_validation_and_complete() -> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-test-workspace")?;
    let instance = TempDir::new("maestria-test-instance")?;
    let ip = instance.path().to_string_lossy();
    let wp = workspace.path().to_string_lossy();
    assert_init_ok(ip.as_ref(), wp.as_ref())?;
    write_file(
        workspace.path(),
        "notes.md",
        "# Design Notes\n\nThe system uses a distributed ledger for consensus.\n",
    )?;
    let notes = workspace
        .path()
        .join("notes.md")
        .to_string_lossy()
        .into_owned();
    assert_index_ok(ip.as_ref(), &notes)?;
    let (_chunk_id, evidence_id) = assert_search_finds(ip.as_ref(), "distributed")?;
    let task_id = assert_task_start(ip.as_ref(), "Validate and complete")?;
    assert_ok(&[
        "task",
        "add-evidence",
        "-i",
        ip.as_ref(),
        &task_id,
        "--evidence-id",
        &evidence_id,
    ])?;
    let validation_output = assert_ok_lines(
        &["task", "request-validation", "-i", ip.as_ref(), &task_id],
        1,
    )?;
    let report_id =
        parse_kv_value(&validation_output, "report").ok_or("validation output missing report")?;
    let report_id = report_id.parse::<u64>()?;
    let passed =
        parse_kv_value(&validation_output, "passed").ok_or("validation output missing passed")?;
    assert_eq!(
        passed, "true",
        "validation should pass for evidence-backed task: {validation_output}"
    );
    assert_ok(&[
        "task",
        "complete",
        "-i",
        ip.as_ref(),
        &task_id,
        "--report-id",
        &report_id.to_string(),
    ])?;
    let task_line = assert_task_show(ip.as_ref(), &task_id)?;
    assert!(
        task_line.contains("CompletedVerified") || task_line.contains("CompletedWithWarnings"),
        "expected completed status after task completion: {task_line}"
    );
    Ok(())
}
