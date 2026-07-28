use maestria_ports::{HarnessRequest, PortError};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

pub(crate) const MAX_STDOUT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_STDERR_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOTAL_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const READ_BUFFER_BYTES: usize = 8 * 1024;

#[derive(Debug)]
enum DrainError {
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

impl DrainError {
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

fn reserve_total_output(total: &AtomicUsize, amount: usize, limit: usize) -> bool {
    let mut current = total.load(Ordering::Acquire);
    loop {
        let Some(next) = current.checked_add(amount) else {
            return false;
        };
        if next > limit {
            return false;
        }
        match total.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(observed) => current = observed,
        }
    }
}

async fn drain_limited<R: tokio::io::AsyncRead + Unpin>(
    handle: &mut Option<R>,
    stream: &'static str,
    stream_limit: usize,
    total_limit: usize,
    total: &AtomicUsize,
) -> Result<Vec<u8>, DrainError> {
    let Some(reader) = handle.as_mut() else {
        return Ok(Vec::new());
    };

    let mut output = Vec::with_capacity(stream_limit);
    let mut buffer = [0u8; READ_BUFFER_BYTES];

    loop {
        let remaining = stream_limit.saturating_sub(output.len());
        let read_size = if remaining == 0 {
            1
        } else {
            remaining.saturating_add(1).min(READ_BUFFER_BYTES)
        };
        let read = reader
            .read(&mut buffer[..read_size])
            .await
            .map_err(|source| DrainError::Io { stream, source })?;
        if read == 0 {
            return Ok(output);
        }
        if read > remaining {
            return Err(DrainError::StreamLimitExceeded {
                stream,
                limit: stream_limit,
            });
        }
        if !reserve_total_output(total, read, total_limit) {
            return Err(DrainError::TotalLimitExceeded {
                stream,
                limit: total_limit,
            });
        }
        output.extend_from_slice(&buffer[..read]);
    }
}

async fn kill_and_reap(child: &mut Child) {
    if let Err(error) = child.start_kill() {
        tracing::warn!(%error, "failed to kill child process");
    }
    if let Err(error) = child.wait().await {
        tracing::warn!(%error, "failed to reap child process");
    }
}

pub(crate) async fn spawn_and_collect(
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

    let mut child = cmd.spawn().map_err(|error| PortError::InternalContext {
        context: "spawn harness child process",
        source: format!("{program}: {error}"),
    })?;

    let mut stdout_handle = child.stdout.take();
    let mut stderr_handle = child.stderr.take();
    let total_output = Arc::new(AtomicUsize::new(0));
    let work = async {
        let mut stdout_reader = Box::pin(drain_limited(
            &mut stdout_handle,
            "stdout",
            MAX_STDOUT_BYTES,
            MAX_TOTAL_OUTPUT_BYTES,
            total_output.as_ref(),
        ));
        let mut stderr_reader = Box::pin(drain_limited(
            &mut stderr_handle,
            "stderr",
            MAX_STDERR_BYTES,
            MAX_TOTAL_OUTPUT_BYTES,
            total_output.as_ref(),
        ));
        let mut stdout_result = None;
        let mut stderr_result = None;

        while stdout_result.is_none() || stderr_result.is_none() {
            tokio::select! {
                result = &mut stdout_reader, if stdout_result.is_none() => {
                    match result {
                        Ok(output) => stdout_result = Some(Ok(output)),
                        Err(error) => {
                            kill_and_reap(&mut child).await;
                            return Err(error.into_port_error());
                        }
                    }
                }
                result = &mut stderr_reader, if stderr_result.is_none() => {
                    match result {
                        Ok(output) => stderr_result = Some(Ok(output)),
                        Err(error) => {
                            kill_and_reap(&mut child).await;
                            return Err(error.into_port_error());
                        }
                    }
                }
            }
        }

        let Some(stdout_result) = stdout_result else {
            return Err(PortError::InternalContext {
                context: "stdout collector completed without a result",
                source: "collector state invariant violated".to_string(),
            });
        };
        let Some(stderr_result) = stderr_result else {
            return Err(PortError::InternalContext {
                context: "stderr collector completed without a result",
                source: "collector state invariant violated".to_string(),
            });
        };
        let stdout = stdout_result.map_err(DrainError::into_port_error)?;
        let stderr = stderr_result.map_err(DrainError::into_port_error)?;
        let status = child
            .wait()
            .await
            .map_err(|source| PortError::InternalContext {
                context: "child process wait error",
                source: source.to_string(),
            })?;
        Ok((status, stdout, stderr))
    };

    match tokio::time::timeout(request.duration_budget, work).await {
        Ok(result) => result,
        Err(_elapsed) => {
            kill_and_reap(&mut child).await;
            Err(PortError::InternalContext {
                context: "harness process execution timed out",
                source: format!("{program} exceeded budget {:?}", request.duration_budget),
            })
        }
    }
}
