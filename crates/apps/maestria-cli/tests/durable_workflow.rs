use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
fn bin() -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::env::var("CARGO_BIN_EXE_maestria-cli")?)
}
fn run(args: &[&str]) -> Result<(i32, String, String), Box<dyn std::error::Error>> {
    let output = Command::new(bin()?).args(args).output()?;
    let code = if output.status.success() { 0 } else { 1 };
    Ok((
        code,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}
struct TempDir(PathBuf);
impl TempDir {
    fn new(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let base = std::env::temp_dir();
        for n in 0..1000 {
            let path = base.join(format!("{prefix}-{n}"));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(format!("could not create temp dir under {}", base.display()).into())
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn write_file(parent: &Path, name: &str, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = parent.join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, contents)?;
    Ok(())
}
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
    let page_obj       = b"3 0 obj\n<< /Type /Page /Parent 4 0 R /MediaBox [0 0 612 792] /Contents 2 0 R /Resources << /Font << /F1 1 0 R >> >> >>\nendobj\n";
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
fn assert_ok(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(args)?;
    assert_eq!(
        code, 0,
        "command failed: {:?}\nstdout: {stdout}\nstderr: {stderr}",
        args
    );
    Ok(stdout)
}
fn assert_ok_lines(
    args: &[&str],
    expected_lines: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let stdout = assert_ok(args)?;
    let actual_lines = stdout.lines().filter(|line| !line.is_empty()).count();
    assert_eq!(
        actual_lines, expected_lines,
        "unexpected stdout line count for {:?}: {stdout}",
        args
    );
    Ok(stdout)
}
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
fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .collect()
}
fn assert_init_ok(instance_path: &str, read_root: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["init", "-i", instance_path, "--read-root", read_root], 2)?;
    assert!(
        stdout.contains("initialized"),
        "init stdout missing 'initialized': {stdout}"
    );
    assert!(
        stdout.contains("manifest"),
        "init stdout missing 'manifest': {stdout}"
    );
    Ok(())
}
fn assert_index_ok(instance_path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["index", "-i", instance_path, file], 1)?;
    assert!(
        stdout.contains("indexed "),
        "expected 'indexed' in index output: {stdout}"
    );
    Ok(())
}
fn assert_reindex_unchanged(
    instance_path: &str,
    file: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["index", "-i", instance_path, file], 1)?;
    assert!(
        stdout.contains("unchanged "),
        "expected 'unchanged' in re-index output: {stdout}"
    );
    Ok(())
}
fn assert_search_finds(
    instance_path: &str,
    query: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["search", "-i", instance_path, query], 2)?;
    let chunk_output_line = stdout
        .lines()
        .find(|line| line.contains("chunk="))
        .ok_or("search output missing chunk line")?;
    let kv = parse_kv(chunk_output_line);
    let chunk_id_str = kv
        .iter()
        .find(|(k, _)| *k == "chunk")
        .map(|(_, v)| *v)
        .ok_or("search output missing chunk=<id>")?;
    assert!(
        chunk_id_str.parse::<u64>().is_ok(),
        "chunk id not a u64: {chunk_id_str}"
    );
    let evidence_id_str = kv
        .iter()
        .find(|(k, _)| *k == "evidence")
        .map(|(_, v)| *v)
        .ok_or("search output missing evidence=<id>")?;
    assert!(
        evidence_id_str.parse::<u64>().is_ok(),
        "evidence id not a u64: {evidence_id_str}"
    );
    Ok((chunk_id_str.to_string(), evidence_id_str.to_string()))
}
fn status_event_count(instance_path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(&["status", "-i", instance_path])?;
    assert_eq!(code, 0, "status failed: {stderr}");
    Ok(stdout
        .lines()
        .find_map(|line| line.strip_prefix("events "))
        .and_then(|value| value.parse().ok())
        .map_or(0, |value| value))
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
fn assert_reject_outside(
    instance_path: &str,
    file: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let err = assert_err(&["index", "-i", instance_path, file])?;
    assert!(
        err.contains("outside the instance read scope") || err.contains("excluded by policy"),
        "expected scope rejection, got: {err}"
    );
    Ok(())
}
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
    assert_reject_outside(ip.as_ref(), &sneaky)?;
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
    let stdout = assert_ok_lines(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().to_string_lossy(),
            "--recursive",
        ],
        1,
    )?;
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
    let stdout = assert_ok_lines(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().join("paper.pdf").to_string_lossy(),
        ],
        1,
    )?;
    assert!(
        stdout.contains("indexed "),
        "expected 'indexed' in index output: {stdout}"
    );
    let stdout = assert_ok_lines(
        &[
            "search",
            "-i",
            &instance.path().to_string_lossy(),
            "distributed",
        ],
        2,
    )?;
    let search_lines: Vec<&str> = stdout.lines().collect();
    assert!(
        search_lines
            .first()
            .is_some_and(|line| line.starts_with("card ")),
        "card result must render before chunks: {stdout}"
    );
    assert!(
        search_lines
            .get(1)
            .is_some_and(|line| line.contains("chunk=") && line.contains("evidence=")),
        "chunk result must follow card with evidence id: {stdout}"
    );
    let chunk_output_line = stdout
        .lines()
        .find(|line| line.contains("chunk="))
        .ok_or("search output missing chunk line")?;
    let kv = parse_kv(chunk_output_line);
    let chunk_id_str = kv
        .iter()
        .find(|(k, _)| *k == "chunk")
        .map(|(_, v)| *v)
        .ok_or("search output missing chunk=<id>")?;
    assert!(
        chunk_id_str.parse::<u64>().is_ok(),
        "chunk id not a u64: {chunk_id_str}"
    );
    let stdout = assert_ok_lines(
        &[
            "open-evidence",
            "-i",
            &instance.path().to_string_lossy(),
            "--chunk-id",
            chunk_id_str,
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
    let err = assert_err(&[
        "index",
        "-i",
        &instance.path().to_string_lossy(),
        &workspace.path().join("scanned.pdf").to_string_lossy(),
    ])?;
    assert!(
        err.contains("timeout") || err.contains("parser failed"),
        "expected timeout or parser failure for no-text PDF, got: {err}"
    );
    let stdout = assert_ok_lines(
        &[
            "search",
            "-i",
            &instance.path().to_string_lossy(),
            "anything",
        ],
        0,
    )?;
    assert!(
        stdout.trim().is_empty(),
        "expected no search results for failed PDF, got: {stdout}"
    );
    Ok(())
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
