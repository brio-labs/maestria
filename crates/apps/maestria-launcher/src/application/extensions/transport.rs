use std::collections::BTreeSet;
use std::process::Stdio;
use std::time::Duration;

use maestria_extensions::{
    HostMessage, MAX_ACTIVE_REQUESTS_PER_COMMAND, MAX_JSON_LINE_BYTES, ProtocolError, WorkerMessage,
};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::task::JoinHandle;
use tokio::time::timeout;

const FRAME_DEADLINE: Duration = Duration::from_secs(10);
const WRITE_DEADLINE: Duration = Duration::from_secs(2);
const MAX_STDERR_BYTES: usize = 8192;

#[derive(Debug, Error)]
pub(super) enum TransportError {
    #[error("extension pipe I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid extension protocol: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("extension worker timed out")]
    TimedOut,
    #[error("extension worker exited before completing its command: {0}")]
    Exited(String),
    #[error("extension worker sent a message for an inactive command")]
    WrongCommand,
    #[error("extension worker exceeded or reused a pending request ID")]
    RequestLimit,
}

pub(super) struct WorkerSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: JoinHandle<std::io::Result<String>>,
    active_command: Option<String>,
    pending: BTreeSet<String>,
}

impl WorkerSession {
    pub(super) fn spawn(mut command: Command) -> Result<Self, TransportError> {
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.kill_on_drop(true);
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("worker stdin missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("worker stdout missing"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| std::io::Error::other("worker stderr missing"))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr: tokio::spawn(read_stderr(stderr)),
            active_command: None,
            pending: BTreeSet::new(),
        })
    }

    pub(super) async fn send(&mut self, message: &HostMessage) -> Result<(), TransportError> {
        match message {
            HostMessage::CommandInvoke { .. } | HostMessage::ActionInvoke { .. }
                if self.active_command.is_none() => {}
            HostMessage::CapabilityResponse { request_id, .. }
                if self.pending.contains(request_id) => {}
            HostMessage::CommandCancel { command_id, .. }
                if self.active_command.as_deref() == Some(command_id) => {}
            _ => return Err(TransportError::WrongCommand),
        }
        let bytes = message.encode_line()?;
        timeout(WRITE_DEADLINE, self.stdin.write_all(&bytes))
            .await
            .map_err(|_| TransportError::TimedOut)??;
        match message {
            HostMessage::CommandInvoke { command_id, .. }
            | HostMessage::ActionInvoke { command_id, .. } => {
                self.active_command = Some(command_id.clone());
            }
            HostMessage::CapabilityResponse { request_id, .. } => {
                self.pending.remove(request_id);
            }
            HostMessage::CommandCancel { .. } => {}
        }
        Ok(())
    }

    pub(super) async fn receive(&mut self) -> Result<WorkerMessage, TransportError> {
        let line = timeout(FRAME_DEADLINE, self.read_line())
            .await
            .map_err(|_| TransportError::TimedOut)??;
        let message = WorkerMessage::parse_line(&line)?;
        let command_id = match &message {
            WorkerMessage::ViewUpdate { command_id, .. }
            | WorkerMessage::CapabilityRequest { command_id, .. }
            | WorkerMessage::CommandComplete { command_id, .. } => command_id,
        };
        if self.active_command.as_deref() != Some(command_id) {
            return Err(TransportError::WrongCommand);
        }
        match &message {
            WorkerMessage::CapabilityRequest { request_id, .. } => {
                if self.pending.len() >= MAX_ACTIVE_REQUESTS_PER_COMMAND
                    || !self.pending.insert(request_id.clone())
                {
                    return Err(TransportError::RequestLimit);
                }
            }
            WorkerMessage::CommandComplete { .. } => {
                if !self.pending.is_empty() {
                    return Err(TransportError::RequestLimit);
                }
                self.active_command = None;
            }
            WorkerMessage::ViewUpdate { .. } => {}
        }
        Ok(message)
    }

    async fn read_line(&mut self) -> Result<Vec<u8>, TransportError> {
        let mut line = Vec::with_capacity(4096);
        loop {
            let buffered = self.stdout.fill_buf().await?;
            if buffered.is_empty() {
                let status = self.child.wait().await?;
                let stderr = (&mut self.stderr).await.map_err(std::io::Error::other)??;
                return Err(TransportError::Exited(format!("{status}; {stderr}")));
            }
            let segment_end = buffered
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(buffered.len(), |index| index + 1);
            if line.len().saturating_add(segment_end) > MAX_JSON_LINE_BYTES + 1 {
                return Err(TransportError::Protocol(ProtocolError::Oversized));
            }
            line.extend_from_slice(&buffered[..segment_end]);
            self.stdout.consume(segment_end);
            if line.last() == Some(&b'\n') {
                line.pop();
                return Ok(line);
            }
        }
    }
}

impl Drop for WorkerSession {
    fn drop(&mut self) {
        self.stderr.abort();
    }
}

async fn read_stderr(mut stderr: tokio::process::ChildStderr) -> std::io::Result<String> {
    let mut captured = Vec::with_capacity(MAX_STDERR_BYTES);
    let mut buffer = [0u8; 1024];
    let mut truncated = false;
    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let remaining = MAX_STDERR_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    let mut message = String::from_utf8_lossy(&captured).into_owned();
    if truncated {
        message.push_str(" [worker stderr truncated]");
    }
    Ok(message)
}
