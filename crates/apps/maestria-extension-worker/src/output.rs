use std::io;

use maestria_extensions::{MAX_JSON_LINE_BYTES, WorkerMessage};
use tokio::io::AsyncWriteExt;

use crate::error::WorkerError;

pub(crate) async fn write_worker_message(message: &WorkerMessage) -> Result<(), WorkerError> {
    let encoded = serde_json::to_vec(message)?;
    if encoded.len() > MAX_JSON_LINE_BYTES {
        return Err(WorkerError::Protocol(
            maestria_extensions::ProtocolError::Oversized,
        ));
    }
    WorkerMessage::parse_line(&encoded)?;

    let mut stdout = tokio::io::stdout();
    stdout
        .write_all(&encoded)
        .await
        .map_err(|source| WorkerError::Io {
            operation: "writing protocol stdout",
            source,
        })?;
    stdout
        .write_all(b"\n")
        .await
        .map_err(|source| WorkerError::Io {
            operation: "framing protocol stdout",
            source,
        })?;
    stdout.flush().await.map_err(|source| WorkerError::Io {
        operation: "flushing protocol stdout",
        source,
    })?;
    Ok(())
}

pub(crate) fn report_error(error: &WorkerError) -> Result<(), io::Error> {
    let record = ErrorRecord {
        kind: "worker.error",
        code: error.code(),
        message: error.diagnostic(),
    };
    let mut encoded = serde_json::to_vec(&record).map_err(io::Error::other)?;
    encoded.push(b'\n');
    let mut stderr = io::stderr().lock();
    use io::Write;
    stderr.write_all(&encoded)?;
    stderr.flush()
}

#[derive(serde::Serialize)]
struct ErrorRecord {
    kind: &'static str,
    code: &'static str,
    message: String,
}
