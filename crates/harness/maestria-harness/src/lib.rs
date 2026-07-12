use maestria_ports::{
    HarnessAdapter, HarnessCapabilities, HarnessCommandClass, HarnessOutcome, HarnessRequest,
    PortError,
};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
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

    // ── 2. readable-root validation for cat ────────────────────────────
    if program == "cat" {
        for arg in &argv[1..] {
            // ignore flags (args starting with `-` are passed through;
            // the metachar check already blocks `-o+Foo` tricks)
            if arg.starts_with('-') {
                continue;
            }
            validate_readable_path(arg, &request.working_directory, &request.readable_roots)?;
        }
    }

    // ── 3. spawn with kill_on_drop for cancellation safety ─────────────
    let mut child = Command::new(program);
    child
        .args(&argv[1..])
        .current_dir(&request.working_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = child.spawn().map_err(|e| PortError::Internal {
        message: format!("failed to spawn {program}: {e}"),
    })?;

    // ── 4. race output vs timeout ──────────────────────────────────────
    // kill_on_drop(true) above ensures the child is killed when the
    // wait_with_output future is cancelled by the timeout wrapper.
    let output = match tokio::time::timeout(request.duration_budget, child.wait_with_output()).await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(PortError::Internal {
                message: format!("{program}: {e}"),
            });
        }
        Err(_elapsed) => {
            // The timeout fired; the inner future was cancelled, which
            // dropped the Child struct.  kill_on_drop(true) sends SIGKILL
            // and waits (best-effort) in the Drop impl.
            return Err(PortError::Internal {
                message: format!("{program} timed out after {:?}", request.duration_budget),
            });
        }
    };

    // ── 5. assemble outcome ────────────────────────────────────────────
    let duration = match start.elapsed() {
        Ok(d) => d,
        Err(_) => std::time::Duration::ZERO,
    };
    let exit_code = output.status.code().map_or(
        {
            // signal → 128+signal (POSIX convention)
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                output.status.signal().map_or(-1, |s| 128 + s)
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
        stdout: output.stdout,
        stderr: output.stderr,
        duration,
        artifacts_created: vec![],
        diff_summary: None,
        validation_hints: vec![],
    })
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
}
