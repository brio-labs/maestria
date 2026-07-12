use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// Path to the compiled maestria-cli binary, injected by Cargo at test time.
fn bin() -> String {
    std::env::var("CARGO_BIN_EXE_maestria-cli")
        .expect("CARGO_BIN_EXE_maestria-cli not set; run via `cargo test`")
}

/// Run the binary and return (exit_code, stdout, stderr) as separate strings.
fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn maestria-cli");
    let code = if output.status.success() { 0 } else { 1 };
    (
        code,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// Create a unique temporary directory that is cleaned up on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let base = std::env::temp_dir();
        for n in 0..1000 {
            let path = base.join(format!("{prefix}-{n}"));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create temp dir {}: {e}", path.display()),
            }
        }
        panic!("could not create temp dir under {}", base.display());
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

/// Write a file into a directory, creating parents as needed.
fn write_file(parent: &Path, name: &str, contents: &str) {
    let path = parent.join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).expect("create parent dirs");
    }
    fs::write(&path, contents).expect("write test file");
}

/// Write binary bytes into a file, creating parents as needed.
fn write_file_bytes(parent: &Path, name: &str, contents: &[u8]) {
    let path = parent.join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).expect("create parent dirs");
    }
    fs::write(&path, contents).expect("write test file");
}

/// Build a minimal valid PDF containing the given text on one page.
/// Constructs PDF bytes directly — no external PDF crate dependency.
fn create_minimal_pdf(text: &[u8]) -> Vec<u8> {
    let text_str = std::str::from_utf8(text).expect("PDF text must be valid UTF-8");

    // Encode for PDF literal string: escape (, ), and \
    let mut pdf_text = String::with_capacity(text_str.len() + 8);
    for ch in text_str.chars() {
        match ch {
            '(' => pdf_text.push_str("\\("),
            ')' => pdf_text.push_str("\\)"),
            '\\' => pdf_text.push_str("\\\\"),
            _ => pdf_text.push(ch),
        }
    }

    // Content stream — text-drawing operators
    let content_data = format!("BT\n/F1 12 Tf\n72 700 Td\n({pdf_text}) Tj\nET");
    let content_len = content_data.len();

    // PDF body objects
    let font_obj = b"1 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>\nendobj\n";
    let content_header = format!("2 0 obj\n<< /Length {content_len} >>\nstream\n");
    let content_footer = "\nendstream\nendobj\n";
    let page_obj       = b"3 0 obj\n<< /Type /Page /Parent 4 0 R /MediaBox [0 0 612 792] /Contents 2 0 R /Resources << /Font << /F1 1 0 R >> >> >>\nendobj\n";
    let pages_obj = b"4 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n";
    let catalog_obj = b"5 0 obj\n<< /Type /Catalog /Pages 4 0 R >>\nendobj\n";

    let mut buf = Vec::with_capacity(1024);

    // ── header ──
    buf.extend_from_slice(b"%PDF-1.4\n");

    // ── body objects (track offsets for xref) ──
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

    // ── cross-reference table (20-byte entries, CRLF terminated) ──
    let xref_offset = buf.len();
    buf.extend_from_slice(b"xref\n0 6\n");
    buf.extend_from_slice(b"0000000000 65535 f\r\n");
    buf.extend_from_slice(format!("{off_font:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_content:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_page:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_pages:010} 00000 n\r\n").as_bytes());
    buf.extend_from_slice(format!("{off_catalog:010} 00000 n\r\n").as_bytes());

    // ── trailer ──
    buf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 5 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n").as_bytes(),
    );

    buf
}

/// Build a minimal PDF with no extractable text (scanned/image-only page).
/// Constructs PDF bytes directly — no external PDF crate dependency.
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

/// Assert exit success, return stdout.
fn assert_ok(args: &[&str]) -> String {
    let (code, stdout, _stderr) = run(args);
    assert_eq!(code, 0, "command failed: {:?}\nstdout: {stdout}", args);
    stdout
}

/// Assert success with an exact number of non-empty stdout lines.
fn assert_ok_lines(args: &[&str], expected_lines: usize) -> String {
    let stdout = assert_ok(args);
    let actual_lines = stdout.lines().filter(|line| !line.is_empty()).count();
    assert_eq!(
        actual_lines, expected_lines,
        "unexpected stdout line count for {:?}: {stdout}",
        args
    );
    stdout
}

/// Assert exit failure, return stderr (CLI writes errors there).
fn assert_err(args: &[&str]) -> String {
    let (code, stdout, stderr) = run(args);
    assert_ne!(
        code, 0,
        "command unexpectedly succeeded: {:?}\nstdout: {stdout}",
        args
    );
    assert!(
        stdout.trim().is_empty(),
        "failed command wrote unexpected stdout: {stdout}"
    );
    stderr
}

/// Parse `key=value` pairs from a key-value line like `artifact=42 chunks=3`.
fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .collect()
}

#[test]
fn durable_cli_workflow() {
    // ── Setup ───────────────────────────────────────────────────────────────
    let workspace = TempDir::new("maestria-test-workspace");
    let instance = TempDir::new("maestria-test-instance");

    // ── 1. Init ─────────────────────────────────────────────────────────────
    let stdout = assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    );
    assert!(
        stdout.contains("initialized"),
        "init stdout missing 'initialized': {stdout}"
    );
    assert!(
        stdout.contains("manifest"),
        "init stdout missing 'manifest': {stdout}"
    );

    // ── 2. Index a Markdown file ────────────────────────────────────────────
    write_file(
        workspace.path(),
        "notes.md",
        "# Design Notes\n\nThe system uses a distributed ledger for consensus.\n",
    );
    let stdout = assert_ok_lines(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().join("notes.md").to_string_lossy(),
        ],
        1,
    );
    assert!(
        stdout.contains("indexed "),
        "expected 'indexed' in index output: {stdout}"
    );

    // ── 3. Re-index: must report unchanged ──────────────────────────────────
    let stdout = assert_ok_lines(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().join("notes.md").to_string_lossy(),
        ],
        1,
    );
    assert!(
        stdout.contains("unchanged "),
        "expected 'unchanged' in re-index output: {stdout}"
    );

    // ── 4. Restart durability: search via a new process ─────────────────────
    let stdout = assert_ok_lines(
        &[
            "search",
            "-i",
            &instance.path().to_string_lossy(),
            "distributed",
        ],
        1,
    );
    let output_line = stdout
        .lines()
        .next()
        .expect("search output missing result line");
    let kv = parse_kv(output_line);
    let chunk_id_str = kv
        .iter()
        .find(|(k, _)| *k == "chunk")
        .map(|(_, v)| *v)
        .expect("search output missing chunk=<id>");
    assert!(
        chunk_id_str.parse::<u64>().is_ok(),
        "chunk id not a u64: {chunk_id_str}"
    );

    // ── 5. Open evidence by chunk id (separate process) ─────────────────────
    let stdout = assert_ok_lines(
        &[
            "open-evidence",
            "-i",
            &instance.path().to_string_lossy(),
            "--chunk-id",
            chunk_id_str,
        ],
        3,
    );
    // Source path
    assert!(
        stdout.contains("source=file"),
        "open-evidence missing source=file: {stdout}"
    );
    assert!(
        stdout.contains("notes.md"),
        "open-evidence missing source path: {stdout}"
    );
    // Excerpt
    assert!(
        stdout.contains("excerpt="),
        "open-evidence missing excerpt: {stdout}"
    );
    let excerpt_line = stdout
        .lines()
        .find(|line| line.starts_with("excerpt="))
        .expect("excerpt line not found");
    assert!(
        !excerpt_line["excerpt=".len()..].is_empty(),
        "excerpt is empty: {excerpt_line}"
    );
    // Hash
    assert!(
        stdout.contains("hash="),
        "open-evidence missing hash: {stdout}"
    );

    // ── 6. Reject file outside the approved root ────────────────────────────
    let outside = TempDir::new("maestria-test-outside");
    write_file(outside.path(), "sneaky.md", "# sneaky\n");
    let err = assert_err(&[
        "index",
        "-i",
        &instance.path().to_string_lossy(),
        &outside.path().join("sneaky.md").to_string_lossy(),
    ]);
    assert!(
        err.contains("outside the instance read scope") || err.contains("excluded by policy"),
        "expected scope rejection, got: {err}"
    );

    // ── 7. Reject excluded .env file ────────────────────────────────────────
    write_file(workspace.path(), ".env", "SECRET=do_not_index");
    let err = assert_err(&[
        "index",
        "-i",
        &instance.path().to_string_lossy(),
        &workspace.path().join(".env").to_string_lossy(),
    ]);
    assert!(
        err.contains("excluded by privacy policy")
            || err.contains("outside the instance read scope"),
        "expected exclusion rejection for .env, got: {err}"
    );
}

#[test]
fn recursive_index_skips_default_privacy_paths() {
    let workspace = TempDir::new("maestria-test-recursive-workspace");
    let instance = TempDir::new("maestria-test-recursive-instance");

    assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    );
    write_file(workspace.path(), "notes.md", "# Public note\n");
    write_file(
        workspace.path(),
        "credentials/leaked.md",
        "# Sensitive note\n",
    );

    let stdout = assert_ok_lines(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().to_string_lossy(),
            "--recursive",
        ],
        1,
    );
    assert!(
        stdout.contains("notes.md"),
        "public note was not indexed: {stdout}"
    );
    assert!(
        !stdout.contains("leaked.md"),
        "privacy-excluded file was indexed: {stdout}"
    );
}

#[test]
fn query_commands_require_an_initialized_instance() {
    let instance = TempDir::new("maestria-test-uninitialized-instance");

    let error = assert_err(&[
        "search",
        "-i",
        &instance.path().to_string_lossy(),
        "anything",
    ]);
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
    ]);
    assert!(
        error.contains("instance manifest is missing"),
        "unexpected uninitialized-instance evidence error: {error}"
    );
}

#[test]
fn pdf_indexing_workflow() {
    // ── Setup ───────────────────────────────────────────────────────────────
    let workspace = TempDir::new("maestria-test-pdf-workspace");
    let instance = TempDir::new("maestria-test-pdf-instance");

    // ── 1. Init ─────────────────────────────────────────────────────────────
    let stdout = assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    );
    assert!(
        stdout.contains("initialized"),
        "init stdout missing 'initialized': {stdout}"
    );

    // ── 2. Index a valid PDF ────────────────────────────────────────────────
    let pdf_bytes = create_minimal_pdf(b"The system uses a distributed ledger for consensus.");
    write_file_bytes(workspace.path(), "paper.pdf", &pdf_bytes);
    let stdout = assert_ok_lines(
        &[
            "index",
            "-i",
            &instance.path().to_string_lossy(),
            &workspace.path().join("paper.pdf").to_string_lossy(),
        ],
        1,
    );
    assert!(
        stdout.contains("indexed "),
        "expected 'indexed' in index output: {stdout}"
    );

    // ── 3. Restart durability: search for PDF content ───────────────────────
    let stdout = assert_ok_lines(
        &[
            "search",
            "-i",
            &instance.path().to_string_lossy(),
            "distributed",
        ],
        1,
    );
    let output_line = stdout
        .lines()
        .next()
        .expect("search output missing result line");
    let kv = parse_kv(output_line);
    let chunk_id_str = kv
        .iter()
        .find(|(k, _)| *k == "chunk")
        .map(|(_, v)| *v)
        .expect("search output missing chunk=<id>");
    assert!(
        chunk_id_str.parse::<u64>().is_ok(),
        "chunk id not a u64: {chunk_id_str}"
    );

    // ── 4. Open evidence by chunk id ────────────────────────────────────────
    let stdout = assert_ok_lines(
        &[
            "open-evidence",
            "-i",
            &instance.path().to_string_lossy(),
            "--chunk-id",
            chunk_id_str,
        ],
        3,
    );
    // Source label: must show PDF provenance with page numbers
    assert!(
        stdout.contains("source=pdf"),
        "open-evidence missing source=pdf: {stdout}"
    );
    assert!(
        stdout.contains("pages=1-1"),
        "open-evidence missing pages=1-1: {stdout}"
    );
    // Excerpt
    assert!(
        stdout.contains("excerpt="),
        "open-evidence missing excerpt: {stdout}"
    );
    let excerpt_line = stdout
        .lines()
        .find(|line| line.starts_with("excerpt="))
        .expect("excerpt line not found");
    assert!(
        !excerpt_line["excerpt=".len()..].is_empty(),
        "excerpt is empty: {excerpt_line}"
    );
}

#[test]
fn pdf_no_text_is_rejected() {
    // ── Setup ───────────────────────────────────────────────────────────────
    let workspace = TempDir::new("maestria-test-pdf-empty-workspace");
    let instance = TempDir::new("maestria-test-pdf-empty-instance");

    assert_ok_lines(
        &[
            "init",
            "-i",
            &instance.path().to_string_lossy(),
            "--read-root",
            &workspace.path().to_string_lossy(),
        ],
        2,
    );

    // ── Write a PDF with no extractable text ────────────────────────────────
    let empty_pdf = create_no_text_pdf();
    write_file_bytes(workspace.path(), "scanned.pdf", &empty_pdf);

    // ── Index must fail ─────────────────────────────────────────────────────
    let err = assert_err(&[
        "index",
        "-i",
        &instance.path().to_string_lossy(),
        &workspace.path().join("scanned.pdf").to_string_lossy(),
    ]);
    assert!(
        err.contains("timeout") || err.contains("parser failed"),
        "expected timeout or parser failure for no-text PDF, got: {err}"
    );

    // ── Restart: search must find nothing for the failed artifact ───────────
    let stdout = assert_ok_lines(
        &[
            "search",
            "-i",
            &instance.path().to_string_lossy(),
            "anything",
        ],
        0,
    );
    assert!(
        stdout.trim().is_empty(),
        "expected no search results for failed PDF, got: {stdout}"
    );
}
