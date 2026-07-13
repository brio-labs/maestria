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

    if program == "cat" {
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
    }

    let mut cmd = Command::new(program);
    cmd.args(&argv[1..])
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

    let (status, stdout, stderr) = match tokio::time::timeout(request.duration_budget, work).await {
        Ok(Ok(tuple)) => tuple,
        Ok(Err(e)) => return Err(e),
        Err(_elapsed) => {
            if let Ok(Some(s)) = child.try_wait() {
                let (out_r, err_r) =
                    tokio::join!(drain_opt(&mut stdout_handle), drain_opt(&mut stderr_handle),);
                let out = match out_r {
                    Ok(v) => v,
                    Err(_) => Vec::new(),
                };
                let err = match err_r {
                    Ok(v) => v,
                    Err(_) => Vec::new(),
                };
                (s, out, err)
            } else {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(PortError::Internal {
                    message: format!("{program} timed out after {:?}", request.duration_budget),
                });
            }
        }
    };

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

#[cfg(test)]
mod tests;
