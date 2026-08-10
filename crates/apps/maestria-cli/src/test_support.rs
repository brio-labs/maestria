use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

pub fn bin() -> Result<String, Box<dyn std::error::Error>> {
    Ok(std::env::var("CARGO_BIN_EXE_maestria-cli")?)
}

pub fn run(args: &[&str]) -> Result<(i32, String, String), Box<dyn std::error::Error>> {
    run_bounded(args, Duration::from_secs(30))
}

pub fn run_bounded(
    args: &[&str],
    timeout: Duration,
) -> Result<(i32, String, String), Box<dyn std::error::Error>> {
    let mut child = Command::new(bin()?)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let poll_count = timeout.as_millis().div_ceil(10);
    for _ in 0..poll_count {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            let code = if output.status.success() { 0 } else { 1 };
            return Ok((
                code,
                String::from_utf8_lossy(&output.stdout).into_owned(),
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.kill()?;
    let _ = child.wait();
    Err(format!("command timed out: {args:?}").into())
}
pub struct TempDir(PathBuf);
impl TempDir {
    pub fn new(prefix: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let base = std::env::temp_dir();
        for n in 0..1000 {
            let path = base.join(format!("{prefix}-{}-{n}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(format!("could not create temp dir under {}", base.display()).into())
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub fn write_file(
    parent: &Path,
    name: &str,
    contents: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = parent.join(name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&path, contents)?;
    Ok(())
}
pub fn assert_ok(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(args)?;
    assert_eq!(
        code, 0,
        "command failed: {args:?}\nstdout: {stdout}\nstderr: {stderr}"
    );
    Ok(stdout)
}
pub fn assert_ok_lines(
    args: &[&str],
    expected_lines: usize,
) -> Result<String, Box<dyn std::error::Error>> {
    let stdout = assert_ok(args)?;
    let actual_lines = stdout.lines().filter(|line| !line.is_empty()).count();
    assert_eq!(
        actual_lines, expected_lines,
        "unexpected stdout line count for {args:?}: {stdout}"
    );
    Ok(stdout)
}
pub fn assert_init_ok(
    instance_path: &str,
    read_root: &str,
) -> Result<(), Box<dyn std::error::Error>> {
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
pub fn assert_index_ok(instance_path: &str, file: &str) -> Result<(), Box<dyn std::error::Error>> {
    let stdout = assert_ok_lines(&["index", "-i", instance_path, file], 2)?;
    assert!(
        stdout.contains("indexed"),
        "index stdout missing 'indexed': {stdout}"
    );
    Ok(())
}

/// Assert a command fails with a non-zero exit code, writes nothing to
/// stdout, and return its stderr.
pub fn assert_err(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(args)?;
    assert_ne!(
        code, 0,
        "command unexpectedly succeeded: {args:?}\nstdout: {stdout}"
    );
    assert!(
        stdout.trim().is_empty(),
        "failed command wrote unexpected stdout: {stdout}"
    );
    Ok(stderr)
}

/// Like [`assert_err`], but with a caller-supplied wall-clock budget.
///
/// Commands whose own timeout budget is close to the default 30s harness cap
/// (e.g. `index` waiting on a no-text PDF) must be run with a larger budget so
/// the product's own timeout produces the expected error instead of being
/// killed by the harness.
pub fn assert_err_bounded(
    args: &[&str],
    timeout: Duration,
) -> Result<String, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run_bounded(args, timeout)?;
    assert_ne!(
        code, 0,
        "command unexpectedly succeeded: {args:?}\nstdout: {stdout}"
    );
    assert!(
        stdout.trim().is_empty(),
        "failed command wrote unexpected stdout: {stdout}"
    );
    Ok(stderr)
}

/// Parse whitespace-separated `key=value` tokens from a CLI output line.
pub fn parse_kv(line: &str) -> Vec<(&str, &str)> {
    line.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .collect()
}

/// Look up one `key=value` token in a CLI output line.
pub fn parse_kv_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    parse_kv(line)
        .into_iter()
        .find_map(|(candidate_key, value)| (candidate_key == key).then_some(value))
}

/// Run `search` and return the `(artifact, evidence)` ids from the first
/// evidence line of its output.
pub fn assert_search_finds(
    instance_path: &str,
    query: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let stdout = assert_ok(&["search", "-i", instance_path, query])?;
    let evidence_output_line = stdout
        .lines()
        .find(|line| line.contains("evidence="))
        .ok_or("search output missing evidence line")?;
    let kv = parse_kv(evidence_output_line);
    let artifact_id = kv
        .iter()
        .find(|(key, _)| *key == "artifact")
        .map(|(_, value)| *value)
        .ok_or("search output missing artifact=<id>")?;
    let evidence_id = kv
        .iter()
        .find(|(key, _)| *key == "evidence")
        .map(|(_, value)| *value)
        .ok_or("search output missing evidence=<id>")?;
    assert!(
        evidence_id.parse::<u64>().is_ok(),
        "evidence id not a u64: {evidence_id}"
    );
    Ok((artifact_id.to_string(), evidence_id.to_string()))
}

/// Run `task start` and return the extracted numeric task id.
pub fn assert_task_start(
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

/// Run `status` and return the persisted event count; missing counts are
/// reported as a test failure rather than silently treated as zero.
pub fn status_event_count(instance_path: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let (code, stdout, stderr) = run(&["status", "-i", instance_path])?;
    assert_eq!(code, 0, "status failed: {stderr}");
    let event_count = stdout
        .lines()
        .find_map(|line| line.strip_prefix("events "))
        .and_then(|value| value.parse().ok())
        .ok_or("status output missing event count")?;
    Ok(event_count)
}
