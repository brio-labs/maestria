use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::SystemTime;
use tokio::process::Command;

// ── shell metacharacter blocklist ──────────────────────────────────────────

/// Characters that MUST NOT appear in any argv token for shell commands.
const FORBIDDEN_CHARS: &[char] = &[
    '|', '&', ';', '<', '>', '$', '`', '\\', '(', ')', '{', '}', '[', ']', '!', '~', '#', '*', '?',
];

/// Allowed command names.
const ALLOWED_PROGRAMS: &[&str] = &["echo", "pwd", "cat"];

fn reject_metachar(arg: &str) -> Result<(), PortError> {
    if let Some(pos) = arg.find(FORBIDDEN_CHARS) {
        return Err(PortError::InvalidInput {
            message: format!(
                "forbidden metacharacter {:?} at offset {} in {:?}",
                arg.chars().nth(pos).map_or('?', |c| c),
                pos,
                arg
            ),
        });
    }
    Ok(())
}

/// Simple tokeniser with single- and double-quote support.
fn tokenize(raw: &str) -> Result<Vec<String>, PortError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // skip whitespace
        if chars[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let mut token = String::new();
        if chars[i] == '\'' {
            i += 1; // skip opening quote
            while i < chars.len() && chars[i] != '\'' {
                token.push(chars[i]);
                i += 1;
            }
            if i >= chars.len() {
                return Err(PortError::InvalidInput {
                    message: "unterminated single quote".to_string(),
                });
            }
            i += 1; // skip closing quote
        } else if chars[i] == '"' {
            i += 1; // skip opening quote
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    match chars[i] {
                        '"' => token.push('"'),
                        '\\' => token.push('\\'),
                        'n' => token.push('\n'),
                        't' => token.push('\t'),
                        c => token.push(c),
                    }
                } else {
                    token.push(chars[i]);
                }
                i += 1;
            }
            if i >= chars.len() {
                return Err(PortError::InvalidInput {
                    message: "unterminated double quote".to_string(),
                });
            }
            i += 1; // skip closing quote
        } else {
            // unquoted token — stop at next whitespace
            while i < chars.len() && !chars[i].is_ascii_whitespace() {
                token.push(chars[i]);
                i += 1;
            }
        }
        tokens.push(token);
    }
    Ok(tokens)
}

/// Check that `path` lives inside at least one readable root.
///
/// Relative paths are resolved against `cwd`.  We normalize `..` / `.`
/// components manually so nonexistent leaves still pass the containment
/// check (the process itself will report the ENOENT).
fn validate_readable_path(
    raw_path: &str,
    cwd: &Path,
    readable_roots: &[PathBuf],
) -> Result<PathBuf, PortError> {
    let candidate = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        cwd.join(raw_path)
    };

    // Normalize `..` and `.` without touching the filesystem.
    let normalized = normalize_path(&candidate);

    let allowed = readable_roots.iter().any(|root| match root.canonicalize() {
        Ok(cr) => normalized.starts_with(&cr),
        Err(_) => false,
    });

    if !allowed {
        return Err(PortError::InvalidInput {
            message: format!("path {:?} is outside readable roots", raw_path),
        });
    }
    Ok(normalized)
}

/// Remove `.` and `..` components from a path without touching the
/// filesystem.  The result is purely lexical.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components: Vec<std::path::Component<'_>> = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                // Pop the last component unless it's already a ParentDir
                // (which means we're above root — keep it).
                if components
                    .last()
                    .is_some_and(|prev| !matches!(prev, std::path::Component::ParentDir))
                {
                    components.pop();
                } else {
                    components.push(c);
                }
            }
            other => components.push(other),
        }
    }
    components.iter().collect()
}

// ── adapter ────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct LocalShellHarnessAdapter;

impl HarnessAdapter for LocalShellHarnessAdapter {
    fn capabilities(&self) -> Result<HarnessCapabilities, PortError> {
        Ok(HarnessCapabilities {
            command_classes: vec![HarnessCommandClass::Shell],
            write_enabled: false,
            read_enabled: true,
            web_enabled: false,
        })
    }

    fn execute(
        &self,
        request: HarnessRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HarnessOutcome, PortError>> + Send + '_>> {
        Box::pin(execute_impl(request))
    }
}

async fn execute_impl(request: HarnessRequest) -> Result<HarnessOutcome, PortError> {
    let start = SystemTime::now();

    if request.class != HarnessCommandClass::Shell {
        return Err(PortError::Internal {
            message: format!("unsupported harness class: {:?}", request.class),
        });
    }

    // ── 1. tokenise and validate argv grammar ──────────────────────────
    let argv = tokenize(&request.command)?;
    if argv.is_empty() {
        return Err(PortError::InvalidInput {
            message: "command must not be empty".to_string(),
        });
    }

    let program = &argv[0];
    if !ALLOWED_PROGRAMS.contains(&program.as_str()) {
        return Err(PortError::InvalidInput {
            message: format!(
                "program {:?} not allowed; expected one of {:?}",
                program, ALLOWED_PROGRAMS
            ),
        });
    }

    for arg in &argv {
        reject_metachar(arg)?;
    }

    validate_cat_args(program, &argv, &request)?;

    let (status, stdout, stderr) = spawn_and_collect(program, &argv[1..], &request).await?;

    let duration = match start.elapsed() {
        Ok(d) => d,
        Err(_) => std::time::Duration::ZERO,
    };
    let exit_code = status.code().map_or(
        {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map_or(-1, |s| 128 + s)
            }
            #[cfg(not(unix))]
            {
                -1
            }
        },
        |c| c,
    );

    Ok(HarnessOutcome {
        run_id: request.run_id,
        command: request.command,
        exit_code,
        stdout,
        stderr,
        duration,
        artifacts_created: vec![],
        diff_summary: None,
        validation_hints: vec![],
    })
}

async fn drain_opt<R: tokio::io::AsyncRead + Unpin>(
    handle: &mut Option<R>,
) -> Result<Vec<u8>, std::io::Error> {
    match handle.as_mut() {
        Some(r) => {
            let mut buf = Vec::new();
            tokio::io::AsyncReadExt::read_to_end(r, &mut buf).await?;
            Ok(buf)
        }
        None => Ok(Vec::new()),
    }
}

fn filename_matches(name: &str, pattern: &str) -> bool {
    if !pattern.contains('*') && !pattern.contains('?') {
        return name == pattern;
    }
    if pattern == "*" {
        return true;
    }
    glob_match(name, pattern)
}

fn glob_match(name: &str, pattern: &str) -> bool {
    let nc: Vec<char> = name.chars().collect();
    let pc: Vec<char> = pattern.chars().collect();
    let (n, p) = (nc.len(), pc.len());
    let mut dp = vec![vec![false; p + 1]; n + 1];
    dp[0][0] = true;
    for j in 1..=p {
        if pc[j - 1] == '*' {
            dp[0][j] = dp[0][j - 1];
        }
    }
    for i in 1..=n {
        for j in 1..=p {
            if pc[j - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if pc[j - 1] == '?' || pc[j - 1] == nc[i - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[n][p]
}

fn validate_filename_patterns(raw_path: &str, patterns: &[String]) -> Result<(), PortError> {
    if patterns.is_empty() {
        return Ok(());
    }
    let path = Path::new(raw_path);
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        for pattern in patterns {
            if filename_matches(&name, pattern) {
                return Err(PortError::InvalidInput {
                    message: format!("path {raw_path:?} matches blocked pattern {pattern:?}"),
                });
            }
        }
    }
    Ok(())
}

fn validate_cat_args(
    program: &str,
    argv: &[String],
    request: &HarnessRequest,
) -> Result<(), PortError> {
    if program != "cat" {
        return Ok(());
    }
    let mut has_path_arg = false;
    for arg in &argv[1..] {
        if arg.starts_with('-') {
            return Err(PortError::InvalidInput {
                message: format!("cat option {arg:?} not allowed; only path operands"),
            });
        }
        has_path_arg = true;
    }
    if !has_path_arg {
        return Err(PortError::InvalidInput {
            message: "cat requires at least one path operand".to_string(),
        });
    }
    for arg in &argv[1..] {
        let resolved =
            validate_readable_path(arg, &request.working_directory, &request.readable_roots)?;
        let check_path = match std::fs::canonicalize(&resolved) {
            Ok(p) => p,
            Err(_) => resolved,
        };
        let path_str = match check_path.to_str() {
            Some(s) => s,
            None => arg,
        };
        validate_filename_patterns(path_str, &request.blocked_patterns)?;
    }
    Ok(())
}

async fn spawn_and_collect(
    program: &str,
    args: &[String],
    request: &HarnessRequest,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), PortError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(&request.working_directory)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| PortError::Internal {
        message: format!("failed to spawn {program}: {e}"),
    })?;

    let mut stdout_handle = child.stdout.take();
    let mut stderr_handle = child.stderr.take();

    let work = async {
        let (status_res, stdout_buf, stderr_buf) = tokio::join!(
            child.wait(),
            drain_opt(&mut stdout_handle),
            drain_opt(&mut stderr_handle),
        );
        let status = status_res.map_err(|e| PortError::Internal {
            message: format!("{program}: {e}"),
        })?;
        let stdout_buf = stdout_buf.map_err(|e| PortError::Internal {
            message: format!("{program}: stdout read error: {e}"),
        })?;
        let stderr_buf = stderr_buf.map_err(|e| PortError::Internal {
            message: format!("{program}: stderr read error: {e}"),
        })?;
        Ok((status, stdout_buf, stderr_buf))
    };

    match tokio::time::timeout(request.duration_budget, work).await {
        Ok(Ok(tuple)) => Ok(tuple),
        Ok(Err(e)) => Err(e),
        Err(_elapsed) => {
            if let Ok(Some(s)) = child.try_wait() {
                let (out_r, err_r) =
                    tokio::join!(drain_opt(&mut stdout_handle), drain_opt(&mut stderr_handle),);
                let out = out_r.map_err(|e| PortError::Internal {
                    message: format!("{program}: stdout drain error after timeout: {e}"),
                })?;
                let err = err_r.map_err(|e| PortError::Internal {
                    message: format!("{program}: stderr drain error after timeout: {e}"),
                })?;
                Ok((s, out, err))
            } else {
                let _ = child.start_kill();
                let _ = child.wait().await;
                Err(PortError::Internal {
                    message: format!("{program} timed out after {:?}", request.duration_budget),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    async fn echo_returns_stdout() {
        let outcome = adapter()
            .execute(shell_request("echo hello world", 5000))
            .await
            .expect("echo should succeed");
        assert_eq!(outcome.exit_code, 0);
        let stdout = String::from_utf8_lossy(&outcome.stdout);
        assert!(stdout.contains("hello world"), "stdout: {stdout:?}");
    }

    #[tokio::test]
    async fn pwd_returns_working_directory() {
        let mut req = shell_request("pwd", 5000);
        req.working_directory = PathBuf::from("/tmp");
        let outcome = adapter().execute(req).await.expect("pwd should succeed");
        assert_eq!(outcome.exit_code, 0);
        let stdout = String::from_utf8_lossy(&outcome.stdout);
        assert!(stdout.contains("/tmp"), "stdout: {stdout:?}");
    }

    #[tokio::test]
    async fn cat_reads_file_in_readable_root() {
        // Write a temporary file in /tmp, then cat it.
        let path = "/tmp/maestria_harness_cat_test.txt";
        std::fs::write(path, b"meow\n").expect("write temp file");

        let mut req = shell_request(&format!("cat {path}"), 5000);
        req.readable_roots = vec![PathBuf::from("/tmp")];
        let outcome = adapter().execute(req).await.expect("cat should succeed");
        assert_eq!(outcome.exit_code, 0);
        assert_eq!(outcome.stdout, b"meow\n");

        std::fs::remove_file(path).ok();
    }

    // ── nonzero exit ───────────────────────────────────────────────────

    #[tokio::test]
    async fn cat_nonexistent_file_returns_nonzero() {
        let mut req = shell_request("cat /tmp/maestria_nonexistent_xyz", 5000);
        req.readable_roots = vec![PathBuf::from("/tmp")];
        let outcome = adapter()
            .execute(req)
            .await
            .expect("cat nonexistent should run");
        assert_ne!(
            outcome.exit_code, 0,
            "expected nonzero exit for missing file"
        );
    }

    // ── rejected grammar ───────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_unknown_program() {
        let result = adapter().execute(shell_request("ls -la", 5000)).await;
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn rejects_metacharacter_redirect() {
        let result = adapter()
            .execute(shell_request("echo foo > bar", 5000))
            .await;
        assert!(
            matches!(result, Err(PortError::InvalidInput { .. })),
            "expected InvalidInput, got {result:?}"
        );
    }

    #[tokio::test]
    async fn rejects_metacharacter_pipe() {
        let result = adapter()
            .execute(shell_request("echo foo | cat", 5000))
            .await;
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn rejects_metacharacter_dollar() {
        let result = adapter().execute(shell_request("echo $HOME", 5000)).await;
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn rejects_metacharacter_backtick() {
        let result = adapter()
            .execute(shell_request("echo `whoami`", 5000))
            .await;
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn rejects_metacharacter_semicolon() {
        let result = adapter()
            .execute(shell_request("echo a; echo b", 5000))
            .await;
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[tokio::test]
    async fn rejects_metacharacter_ampersand() {
        let result = adapter()
            .execute(shell_request("echo a & echo b", 5000))
            .await;
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    // ── rejected path ──────────────────────────────────────────────────

    #[tokio::test]
    async fn cat_rejects_path_outside_readable_roots() {
        let mut req = shell_request("cat /etc/hostname", 5000);
        req.readable_roots = vec![PathBuf::from("/tmp")];
        let result = adapter().execute(req).await;
        assert!(
            matches!(result, Err(PortError::InvalidInput { .. })),
            "expected InvalidInput for path outside roots, got {result:?}"
        );
    }

    // ── timeout ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn timeout_on_slow_command() {
        // cat /dev/urandom produces endless output → pipe fills → process blocks.
        let mut req = shell_request("cat /dev/urandom", 200);
        req.readable_roots = vec![PathBuf::from("/dev")];
        let result = adapter().execute(req).await;
        assert!(
            matches!(result, Err(PortError::Internal { .. })),
            "expected timeout Internal error, got {result:?}"
        );
    }

    // ── cancellation safety (drop test) ────────────────────────────────

    #[tokio::test]
    async fn cancellation_drops_child_cleanly() {
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
    }

    // ── capabilities contract ──────────────────────────────────────────

    #[tokio::test]
    async fn capabilities_report_shell_only() {
        let caps = adapter().capabilities().expect("capabilities");
        assert!(caps.read_enabled);
        assert!(!caps.write_enabled);
        assert!(!caps.web_enabled);
        assert_eq!(caps.command_classes, vec![HarnessCommandClass::Shell]);
    }

    // ── filename pattern matching tests ────────────────────────────

    #[test]
    fn filename_matches_exact() {
        assert!(filename_matches(".env", ".env"));
        assert!(!filename_matches(".env", "other"));
    }

    #[test]
    fn filename_matches_wildcard_suffix() {
        assert!(filename_matches("secret.key", "*.key"));
        assert!(!filename_matches("key.txt", "*.key"));
    }

    #[test]
    fn filename_matches_wildcard_prefix() {
        assert!(filename_matches(".env.prod", ".env.*"));
        assert!(!filename_matches(".env", ".env.*"));
    }

    #[test]
    fn filename_matches_question_wildcard() {
        assert!(filename_matches("a.key", "?.key"));
        assert!(!filename_matches("ab.key", "?.key"));
    }

    #[tokio::test]
    async fn cat_rejects_blocked_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let keyfile = tmp.path().join("secret.key");
        std::fs::write(&keyfile, b"keydata").unwrap();
        let mut req = shell_request(&format!("cat {}", keyfile.display()), 5000);
        req.readable_roots = vec![tmp.path().to_path_buf()];
        req.blocked_patterns = vec!["*.key".into()];
        assert!(adapter().execute(req).await.is_err());
    }

    #[tokio::test]
    async fn cat_rejects_dotenv_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        let envfile = tmp.path().join(".env");
        std::fs::write(&envfile, b"SECRET=xyz").unwrap();
        let mut req = shell_request(&format!("cat {}", envfile.display()), 5000);
        req.readable_roots = vec![tmp.path().to_path_buf()];
        req.blocked_patterns = vec![".env".into()];
        assert!(adapter().execute(req).await.is_err());
    }
}
