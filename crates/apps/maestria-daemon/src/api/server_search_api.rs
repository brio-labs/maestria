use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

use super::protocol::ClientErrorCode;
use super::protocol_search_api::{
    self, SEARCH_API_PROTOCOL, SEARCH_API_VERSION, SEARCH_API_VERSION_2, SearchApiOperation,
    SearchApiReply, SearchApiRequest, SearchApiResponse,
};
use super::server::{self, ApiContext, InteractiveSearchControl};
use super::{MAX_REQUEST_BYTES, dispatch_search_api};

pub(super) async fn handle_request(
    shutdown: &CancellationToken,
    stream: &mut UnixStream,
    context: Arc<ApiContext>,
    value: serde_json::Value,
) -> Result<()> {
    let mut version = 0;
    if let Some(parsed) = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u16::try_from(version).ok())
    {
        version = parsed;
    }
    let matches_protocol =
        value.get("protocol").and_then(serde_json::Value::as_str) == Some(SEARCH_API_PROTOCOL);
    let matches_version = matches!(version, SEARCH_API_VERSION | SEARCH_API_VERSION_2);
    if !matches_protocol || !matches_version {
        return write_reply_until_shutdown(
            shutdown,
            stream,
            None,
            Some("unsupported search API protocol or version".to_string()),
            Some(ClientErrorCode::ProtocolVersionMismatch),
            SEARCH_API_VERSION_2,
        )
        .await;
    }
    let request = match serde_json::from_value::<SearchApiRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            return write_reply_until_shutdown(
                shutdown,
                stream,
                None,
                Some(format!("invalid search API request: {error}")),
                Some(ClientErrorCode::InvalidInput),
                version,
            )
            .await;
        }
    };
    if version == SEARCH_API_VERSION
        && matches!(
            &request.operation,
            SearchApiOperation::IndexingStatus | SearchApiOperation::InteractiveSearch { .. }
        )
    {
        return write_reply_until_shutdown(
            shutdown,
            stream,
            None,
            Some("this search API operation requires version 2".to_string()),
            Some(ClientErrorCode::ProtocolVersionMismatch),
            version,
        )
        .await;
    }
    if let Err(error) = protocol_search_api::validate_operation(&request.operation) {
        return write_reply_until_shutdown(
            shutdown,
            stream,
            None,
            Some(error.message),
            Some(error.code),
            version,
        )
        .await;
    }
    // Only register a superseding generation after the grant is authenticated in
    // serve_search; an untrusted request must not cancel another client's work.
    let interactive_control = matches!(
        &request.operation,
        SearchApiOperation::InteractiveSearch { .. }
    )
    .then(InteractiveSearchControl::default);
    let result = if let Some(interactive) = interactive_control.as_ref() {
        let signal = interactive.signal.clone();
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                interactive.cancel();
                return Ok(());
            },
            _ = wait_for_client_disconnect(stream) => {
                interactive.cancel();
                return Ok(());
            },
            _ = signal.cancelled() => Err(anyhow!("interactive search was superseded")),
            result = timeout(
                Duration::from_millis(u64::from(maestria_retrieval::INTERACTIVE_MAX_LATENCY_MS)),
                dispatch_search_api(
                    &context,
                    request.consumer_realm,
                    request.credential,
                    request.operation,
                    Some(interactive.clone()),
                ),
            ) => match result {
                Ok(response) => response,
                Err(_) => {
                    interactive.cancel();
                    Err(anyhow!("search service request timed out"))
                }
            },
        }
    } else {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Ok(()),
            _ = wait_for_client_disconnect(stream) => return Ok(()),
            result = timeout(
                Duration::from_secs(30),
                dispatch_search_api(
                    &context,
                    request.consumer_realm,
                    request.credential,
                    request.operation,
                    None,
                ),
            ) => match result {
                Ok(response) => response,
                Err(_) => Err(anyhow!("search service request timed out")),
            },
        }
    };
    let response = match (result, interactive_control.as_ref()) {
        (Ok(_), Some(control)) if control.signal.is_cancelled() => {
            Err(anyhow!("interactive search was superseded"))
        }
        (response, _) => response,
    };
    match response {
        Ok(response) => {
            write_reply_until_shutdown(shutdown, stream, Some(response), None, None, version).await
        }
        Err(error) => {
            write_reply_until_shutdown(
                shutdown,
                stream,
                None,
                Some(error.to_string()),
                Some(server::classify_error(&error.to_string())),
                version,
            )
            .await
        }
    }
}

async fn wait_for_client_disconnect(stream: &mut UnixStream) {
    let mut extra_byte = [0_u8; 1];
    let _ = stream.read(&mut extra_byte).await;
}

async fn write_reply_until_shutdown(
    shutdown: &CancellationToken,
    stream: &mut UnixStream,
    response: Option<SearchApiResponse>,
    error: Option<String>,
    error_code: Option<ClientErrorCode>,
    version: u16,
) -> Result<()> {
    let reply = protocol_search_api::reply(response, error, error_code, version);
    match server::run_until_shutdown(shutdown, write_reply(stream, reply)).await {
        Some(result) => result,
        None => Ok(()),
    }
}

async fn write_reply(stream: &mut UnixStream, reply: SearchApiReply) -> Result<()> {
    let mut bytes = serde_json::to_vec(&reply).context("serialise search API response")?;
    if bytes.len() + 1 > MAX_REQUEST_BYTES {
        let bounded_reply = protocol_search_api::reply(
            None,
            Some("search API response exceeds size limit".to_string()),
            Some(ClientErrorCode::Internal),
            reply.version,
        );
        bytes =
            serde_json::to_vec(&bounded_reply).context("serialise bounded search API response")?;
    }
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .await
        .context("write search API response")
}
