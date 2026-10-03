use anyhow::{Context, Result};
use tokio::{
    io::AsyncWriteExt,
    net::UnixStream,
    time::{Duration, timeout},
};
use tokio_util::sync::CancellationToken;

use super::super::protocol::{
    ClientAuthentication, ClientErrorCode, ClientRequest, read_capped_ndjson_line,
};
use super::super::{ClientReplyOut, MAX_REQUEST_BYTES, dispatch};
use super::{ApiContext, RequestPrincipal, classify_error, run_until_shutdown};

pub(super) async fn handle_connection(
    mut stream: UnixStream,
    context: std::sync::Arc<ApiContext>,
    shutdown: CancellationToken,
) -> Result<()> {
    let line = match run_until_shutdown(
        &shutdown,
        timeout(Duration::from_secs(5), read_capped_ndjson_line(&mut stream)),
    )
    .await
    {
        Some(Ok(Ok(line))) => line,
        Some(Ok(Err(error))) => {
            return write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some(error.to_string()),
                Some(ClientErrorCode::InvalidInput),
            )
            .await;
        }
        Some(Err(_)) => {
            return write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some("request timed out".to_string()),
                Some(ClientErrorCode::InvalidInput),
            )
            .await;
        }
        None => return Ok(()),
    };
    let value = match serde_json::from_slice::<serde_json::Value>(line.trim_ascii()) {
        Ok(value) => value,
        Err(error) => {
            return invalid_request(&shutdown, &mut stream, error.to_string()).await;
        }
    };
    if value.get("protocol").is_some() {
        return super::super::server_search_api::handle_request(
            &shutdown,
            &mut stream,
            context,
            value,
        )
        .await;
    }
    let request = match serde_json::from_value::<ClientRequest>(value) {
        Ok(request) => request,
        Err(error) => {
            return invalid_request(&shutdown, &mut stream, error.to_string()).await;
        }
    };
    let Some(principal) = authenticate(request.authentication, &request.operation, &context.token)
    else {
        return write_reply_until_shutdown(
            &shutdown,
            &mut stream,
            None,
            Some("unauthorized".to_string()),
            Some(ClientErrorCode::Unauthorized),
        )
        .await;
    };
    let response =
        match run_until_shutdown(&shutdown, dispatch(&context, principal, request.operation)).await
        {
            Some(response) => response,
            None => return Ok(()),
        };
    match response {
        Ok(response) => {
            write_reply_until_shutdown(&shutdown, &mut stream, Some(response), None, None).await
        }
        Err(error) => {
            write_reply_until_shutdown(
                &shutdown,
                &mut stream,
                None,
                Some(error.to_string()),
                Some(classify_error(&error.to_string())),
            )
            .await
        }
    }
}

fn authenticate(
    authentication: ClientAuthentication,
    operation: &super::super::ClientOperation,
    instance_token: &str,
) -> Option<RequestPrincipal> {
    match authentication {
        ClientAuthentication::InstanceToken { token } if token == instance_token => {
            Some(RequestPrincipal::Instance)
        }
        ClientAuthentication::FederationGrant {
            consumer_realm,
            credential,
        } if matches!(
            operation,
            super::super::ClientOperation::FederationSearch { .. }
                | super::super::ClientOperation::FederationEvidence { .. }
        ) =>
        {
            Some(RequestPrincipal::Federation {
                consumer_realm,
                credential,
            })
        }
        _ => None,
    }
}

async fn invalid_request(
    shutdown: &CancellationToken,
    stream: &mut UnixStream,
    message: String,
) -> Result<()> {
    write_reply_until_shutdown(
        shutdown,
        stream,
        None,
        Some(format!("invalid request: {message}")),
        Some(ClientErrorCode::InvalidInput),
    )
    .await
}

async fn write_reply_until_shutdown(
    shutdown: &CancellationToken,
    stream: &mut UnixStream,
    response: Option<super::super::ClientResponse>,
    error: Option<String>,
    error_code: Option<ClientErrorCode>,
) -> Result<()> {
    match run_until_shutdown(shutdown, write_reply(stream, response, error, error_code)).await {
        Some(result) => result,
        None => Ok(()),
    }
}

async fn write_reply(
    stream: &mut UnixStream,
    response: Option<super::super::ClientResponse>,
    error: Option<String>,
    error_code: Option<ClientErrorCode>,
) -> Result<()> {
    let code = error_code.or_else(|| error.as_deref().map(classify_error));
    let mut bytes = serde_json::to_vec(&ClientReplyOut {
        response,
        error,
        error_code: code,
    })
    .context("serialise daemon response")?;
    if bytes.len() + 1 > MAX_REQUEST_BYTES {
        bytes = serde_json::to_vec(&ClientReplyOut {
            response: None,
            error: Some("daemon response exceeds size limit".to_string()),
            error_code: Some(ClientErrorCode::Internal),
        })
        .context("serialise bounded daemon response")?;
    }
    bytes.push(b'\n');
    stream
        .write_all(&bytes)
        .await
        .context("write daemon response")
}
