use maestria_ports::{HarnessRequest, PortError};
use tokio::process::Command;

pub(crate) async fn drain_opt<R: tokio::io::AsyncRead + Unpin>(
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
                    tokio::join!(drain_opt(&mut stdout_handle), drain_opt(&mut stderr_handle));
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
