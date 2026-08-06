use super::command::{AuthorizedPaths, normalize_path, validate_filename_patterns};
use maestria_ports::{HarnessRequest, PortError};
use std::path::Path;
use tokio::io::AsyncReadExt;

#[cfg(target_os = "linux")]
mod pinned_file;
#[cfg(target_os = "linux")]
use pinned_file::{ensure_regular_file, fd_identity, open_beneath};

pub(crate) const MAX_STDOUT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_STDERR_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOTAL_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Debug)]
enum OutputError {
    Io {
        stream: &'static str,
        source: std::io::Error,
    },
    StreamLimitExceeded {
        stream: &'static str,
        limit: usize,
    },
    TotalLimitExceeded {
        stream: &'static str,
        limit: usize,
    },
}

impl OutputError {
    fn into_port_error(self) -> PortError {
        match self {
            Self::Io { stream, source } => PortError::InternalContext {
                context: "harness process output read error",
                source: format!("{stream}: {source}"),
            },
            Self::StreamLimitExceeded { stream, limit } => PortError::InternalContext {
                context: "harness process output limit exceeded",
                source: format!("{stream} output exceeded stream limit of {limit} bytes"),
            },
            Self::TotalLimitExceeded { stream, limit } => PortError::InternalContext {
                context: "harness process output limit exceeded",
                source: format!(
                    "total output limit of {limit} bytes exceeded while reading {stream}"
                ),
            },
        }
    }
}

fn reserve_total_output(total: &mut usize, amount: usize) -> Result<(), OutputError> {
    let Some(next) = total.checked_add(amount) else {
        return Err(OutputError::TotalLimitExceeded {
            stream: "stdout",
            limit: MAX_TOTAL_OUTPUT_BYTES,
        });
    };
    if next > MAX_TOTAL_OUTPUT_BYTES {
        return Err(OutputError::TotalLimitExceeded {
            stream: "stdout",
            limit: MAX_TOTAL_OUTPUT_BYTES,
        });
    }
    *total = next;
    Ok(())
}

async fn read_limited(
    file: &mut tokio::fs::File,
    stream: &'static str,
    output: &mut Vec<u8>,
    total: &mut usize,
) -> Result<(), OutputError> {
    let stream_limit = match stream {
        "stdout" => MAX_STDOUT_BYTES,
        "stderr" => MAX_STDERR_BYTES,
        _ => 0,
    };
    let mut buffer = [0u8; READ_BUFFER_BYTES];
    loop {
        let remaining = stream_limit.saturating_sub(output.len());
        let read_size = if remaining == 0 {
            1
        } else {
            remaining.saturating_add(1).min(READ_BUFFER_BYTES)
        };
        let read = file
            .read(&mut buffer[..read_size])
            .await
            .map_err(|source| OutputError::Io { stream, source })?;
        if read == 0 {
            return Ok(());
        }
        if read > remaining {
            return Err(OutputError::StreamLimitExceeded {
                stream,
                limit: stream_limit,
            });
        }
        reserve_total_output(total, read)?;
        output.extend_from_slice(&buffer[..read]);
    }
}

fn append_stderr(stderr: &mut Vec<u8>, total: &mut usize, message: &[u8]) -> Result<(), PortError> {
    if stderr.len().saturating_add(message.len()) > MAX_STDERR_BYTES {
        return Err(PortError::InternalContext {
            context: "harness process output limit exceeded",
            source: format!("stderr output exceeded stream limit of {MAX_STDERR_BYTES} bytes"),
        });
    }
    reserve_total_output(total, message.len()).map_err(OutputError::into_port_error)?;
    stderr.extend_from_slice(message);
    Ok(())
}

enum PinnedOperand {
    File { raw: String, file: std::fs::File },
    Missing(String),
}

fn is_environment_path(path: &Path) -> bool {
    path.starts_with("/proc")
        && path
            .components()
            .any(|component| component.as_os_str() == "environ")
}

fn environment_denied(raw: &str) -> Result<(), PortError> {
    if is_environment_path(Path::new(raw)) {
        return Err(PortError::InvalidInputContext {
            context: "cat environment disclosure denied",
            source: raw.to_string(),
        });
    }
    Ok(())
}

fn missing_operand(raw: &str, error: &std::io::Error) -> PinnedOperand {
    PinnedOperand::Missing(format!("cat: {raw}: {error}\n"))
}

#[cfg(target_os = "linux")]
fn validate_opened_path_policy(
    identity: &Path,
    root_handle: &std::fs::File,
    request: &HarnessRequest,
    authorization: &AuthorizedPaths,
    raw: &str,
) -> Result<(), PortError> {
    environment_denied(&identity.to_string_lossy())?;
    if authorization
        .blocked_paths
        .iter()
        .any(|blocked| identity.starts_with(blocked))
    {
        return Err(PortError::InvalidInputContext {
            context: "opened cat path violates readable policy",
            source: format!("{raw:?} -> {identity:?}"),
        });
    }
    let root_identity = fd_identity(root_handle)?;
    identity
        .strip_prefix(&root_identity)
        .map_err(|_| PortError::InternalContext {
            context: "verify opened file is beneath retained root",
            source: format!("{identity:?} is not beneath {root_identity:?}"),
        })?;
    for blocked_handle in &authorization.blocked_path_handles {
        let blocked_identity = fd_identity(blocked_handle)?;
        if identity.starts_with(&blocked_identity) {
            return Err(PortError::InvalidInputContext {
                context: "opened cat path violates readable policy",
                source: format!("{raw:?} -> {identity:?}"),
            });
        }
    }
    for (blocked_root, blocked_paths) in authorization
        .readable_root_handles
        .iter()
        .zip(authorization.blocked_relative_paths.iter())
    {
        let blocked_root_identity = fd_identity(blocked_root)?;
        if identity
            .strip_prefix(blocked_root_identity)
            .is_ok_and(|relative| {
                blocked_paths
                    .iter()
                    .any(|blocked| relative.starts_with(blocked))
            })
        {
            return Err(PortError::InvalidInputContext {
                context: "opened cat path violates readable policy",
                source: format!("{raw:?} -> {identity:?}"),
            });
        }
    }
    validate_filename_patterns(&identity.to_string_lossy(), &request.blocked_patterns)
}

#[cfg(target_os = "linux")]
fn pin_operand(
    raw: &str,
    request: &HarnessRequest,
    authorization: &AuthorizedPaths,
) -> Result<PinnedOperand, PortError> {
    let candidate = if Path::new(raw).is_absolute() {
        normalize_path(Path::new(raw))
    } else {
        normalize_path(&authorization.working_directory.join(raw))
    };
    environment_denied(raw)?;
    environment_denied(&candidate.to_string_lossy())?;

    for (root_index, (root, root_handle)) in authorization
        .readable_roots
        .iter()
        .zip(authorization.readable_root_handles.iter())
        .enumerate()
    {
        if !candidate.starts_with(root) {
            continue;
        }
        let relative = match candidate.strip_prefix(root) {
            Ok(relative) => relative,
            Err(_) => {
                return Err(PortError::InvalidInputContext {
                    context: "cat path escapes readable roots",
                    source: raw.to_string(),
                });
            }
        };
        if authorization.blocked_relative_paths[root_index]
            .iter()
            .any(|blocked| relative.starts_with(blocked))
        {
            return Err(PortError::InvalidInputContext {
                context: "cat path violates readable policy",
                source: raw.to_string(),
            });
        }
        let file = match open_beneath(root_handle, relative) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(missing_operand(raw, &error));
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(code) if code == libc::ENOSYS || code == libc::EOPNOTSUPP
                ) =>
            {
                return Err(PortError::InternalContext {
                    context: "secure harness file handles unavailable",
                    source: error.to_string(),
                });
            }
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(code) if code == libc::EXDEV || code == libc::ELOOP
                ) =>
            {
                return Err(PortError::InvalidInputContext {
                    context: "cat path escapes readable roots",
                    source: raw.to_string(),
                });
            }
            Err(error) => return Ok(missing_operand(raw, &error)),
        };
        if let Err(error) = ensure_regular_file(&file) {
            return Ok(missing_operand(raw, &error));
        }
        let identity = fd_identity(&file)?;
        validate_opened_path_policy(&identity, root_handle, request, authorization, raw)?;
        return Ok(PinnedOperand::File {
            raw: raw.to_string(),
            file,
        });
    }

    Err(PortError::InvalidInputContext {
        context: "cat path outside readable roots",
        source: raw.to_string(),
    })
}

#[cfg(not(target_os = "linux"))]
fn pin_operand(
    _raw: &str,
    _request: &HarnessRequest,
    _authorization: &AuthorizedPaths,
) -> Result<PinnedOperand, PortError> {
    Err(PortError::InternalContext {
        context: "secure harness file handles unavailable",
        source: "cat is disabled on this platform".to_string(),
    })
}

async fn execute_cat(
    args: &[String],
    request: &HarnessRequest,
    authorization: &AuthorizedPaths,
) -> Result<(i32, Vec<u8>, Vec<u8>), PortError> {
    // Pin and verify every existing operand before materializing any output.
    let mut operands = Vec::with_capacity(args.len());
    for raw in args {
        operands.push(pin_operand(raw, request, authorization)?);
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut total = 0usize;
    let mut exit_code = 0;
    for operand in operands {
        match operand {
            PinnedOperand::Missing(message) => {
                exit_code = 1;
                append_stderr(&mut stderr, &mut total, message.as_bytes())?;
            }
            PinnedOperand::File { raw, file } => {
                let mut file = tokio::fs::File::from_std(file);
                match read_limited(&mut file, "stdout", &mut stdout, &mut total).await {
                    Ok(()) => {}
                    Err(OutputError::Io { source, .. }) => {
                        exit_code = 1;
                        let message = format!("cat: {raw}: {source}\n");
                        append_stderr(&mut stderr, &mut total, message.as_bytes())?;
                    }
                    Err(error) => return Err(error.into_port_error()),
                }
            }
        }
    }
    Ok((exit_code, stdout, stderr))
}

async fn execute_inner(
    program: &str,
    args: &[String],
    request: &HarnessRequest,
    authorization: &AuthorizedPaths,
) -> Result<(i32, Vec<u8>, Vec<u8>), PortError> {
    match program {
        "echo" => {
            let output_len = args
                .iter()
                .map(|arg| arg.len())
                .sum::<usize>()
                .saturating_add(args.len().saturating_sub(1))
                .saturating_add(1);
            if output_len > MAX_STDOUT_BYTES || output_len > MAX_TOTAL_OUTPUT_BYTES {
                return Err(PortError::InternalContext {
                    context: "harness process output limit exceeded",
                    source: format!(
                        "stdout output exceeded stream limit of {MAX_STDOUT_BYTES} bytes"
                    ),
                });
            }
            let mut stdout = Vec::with_capacity(output_len);
            for (index, arg) in args.iter().enumerate() {
                if index != 0 {
                    stdout.push(b' ');
                }
                stdout.extend_from_slice(arg.as_bytes());
            }
            stdout.push(b'\n');
            Ok((0, stdout, Vec::new()))
        }
        "pwd" => {
            let mut stdout = authorization
                .working_directory
                .to_string_lossy()
                .into_owned()
                .into_bytes();
            stdout.push(b'\n');
            Ok((0, stdout, Vec::new()))
        }
        "cat" => execute_cat(args, request, authorization).await,
        _ => Err(PortError::InvalidInputContext {
            context: "program not allowed",
            source: program.to_string(),
        }),
    }
}

pub(crate) async fn execute_command(
    program: &str,
    args: &[String],
    request: &HarnessRequest,
    authorization: &AuthorizedPaths,
) -> Result<(i32, Vec<u8>, Vec<u8>), PortError> {
    match tokio::time::timeout(
        request.duration_budget,
        execute_inner(program, args, request, authorization),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(PortError::InternalContext {
            context: "harness process execution timed out",
            source: format!("{program} exceeded budget {:?}", request.duration_budget),
        }),
    }
}
