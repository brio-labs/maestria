use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_daemon::api::DaemonClient;
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    time::{Duration, timeout},
};

use crate::agent::{AgentHost, AgentProfile};
const MAX_HTTP_BODY: usize = 1024 * 1024;
#[path = "http_ask.rs"]
mod http_ask;
#[path = "http_routes.rs"]
mod http_routes;

#[derive(Debug, Clone)]
pub(crate) struct StudioState {
    pub(crate) client: DaemonClient,
    pub(crate) agent: AgentHost,
    pub(crate) bearer: Arc<str>,
    pub(crate) origin: Arc<str>,
    pub(crate) request_slots: Arc<Semaphore>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug, Serialize)]
struct BootstrapResponse<S, N> {
    status: S,
    notebooks: N,
    agents: Vec<AgentProfile>,
}
pub(crate) async fn serve(listener: TcpListener, state: StudioState) -> Result<()> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .context("accept Studio connection")?;
        let connection_state = state.clone();
        tokio::spawn(async move {
            let _ = handle_http(stream, connection_state).await;
        });
    }
}

async fn handle_http(mut stream: TcpStream, state: StudioState) -> Result<()> {
    let permit = state
        .request_slots
        .clone()
        .acquire_owned()
        .await
        .context("acquire Studio request slot")?;
    let request_result = match timeout(Duration::from_secs(5), read_http_request(&mut stream)).await
    {
        Ok(result) => result,
        Err(_) => Err(anyhow!("HTTP request timed out")),
    };
    let request = match request_result {
        Ok(request) => request,
        Err(error) => {
            let (status, code, message) = classify_http_error(&error.to_string());
            let body = serde_json::to_vec(&ErrorResponse {
                error: ErrorDetail { code, message },
            })
            .context("encode Studio request error")?;
            write_http(&mut stream, status, "application/json", &body).await?;
            drop(permit);
            return Ok(());
        }
    };
    if !origin_allowed(
        &request.method,
        request.headers.get("origin"),
        state.origin.as_ref(),
    ) {
        let body =
            br#"{"error":{"code":"origin_denied","message":"Studio request origin is not allowed"}}"#;
        write_http(&mut stream, 403, "application/json", body).await?;
        drop(permit);
        return Ok(());
    }
    let authorized =
        request.headers.get("authorization") == Some(&format!("Bearer {}", state.bearer));
    if !authorized && !is_static_frontend_get(&request.method, &request.path) {
        let body =
            br#"{"error":{"code":"unauthorized","message":"Studio session is unauthorized"}}"#;
        write_http(&mut stream, 401, "application/json", body).await?;
        drop(permit);
        return Ok(());
    }
    let response = http_routes::route(&state, &request.method, &request.path, &request.body).await;
    match response {
        Ok((status, content_type, body)) => {
            write_http(&mut stream, status, content_type, &body).await?
        }
        Err(error) => {
            let (status, code, safe_message) = classify_http_error(&error.to_string());
            let body = serde_json::to_vec(&ErrorResponse {
                error: ErrorDetail {
                    code,
                    message: safe_message,
                },
            })
            .context("encode Studio error")?;
            write_http(&mut stream, status, "application/json", &body).await?;
        }
    }
    drop(permit);
    Ok(())
}

fn is_static_frontend_get(method: &str, path: &str) -> bool {
    method == "GET" && !path.starts_with("/api/")
}

fn origin_allowed(method: &str, origin: Option<&String>, expected: &str) -> bool {
    matches!(method, "GET" | "HEAD") || origin.is_some_and(|value| value == expected)
}

fn classify_http_error(message: &str) -> (u16, &'static str, &'static str) {
    let lower = message.to_ascii_lowercase();
    if lower.contains("source not selected") || lower.contains("source_not_selected") {
        return (
            403,
            "source_not_selected",
            "Evidence is not selected in this notebook",
        );
    }
    if lower.contains("source unavailable") || lower.contains("source_unavailable") {
        return (
            409,
            "source_unavailable",
            "A selected source is unavailable",
        );
    }
    if lower.contains("no evidence") || lower.contains("no_evidence") {
        return (422, "no_evidence", "No grounded evidence was found");
    }
    if lower.contains("agent is unconfigured") || lower.contains("agent_unconfigured") {
        return (
            502,
            "agent_unconfigured",
            "The configured Studio agent is unavailable",
        );
    }
    if lower.contains("agent output") || lower.contains("invalid agent") {
        return (
            502,
            "invalid_agent_output",
            "The configured agent returned an invalid response",
        );
    }
    if lower.contains("acp") {
        return (
            502,
            "agent_protocol",
            "The configured agent could not complete the ACP exchange",
        );
    }
    if lower.contains("method not allowed") || lower.contains("decode") {
        return (400, "invalid_input", "The request is invalid");
    }
    if lower.contains("unauthorized") || lower.contains("access denied") {
        (401, "unauthorized", "Studio session is unauthorized")
    } else if lower.contains("revision") && lower.contains("conflict") {
        (
            409,
            "revision_conflict",
            "The resource changed; reload and retry",
        )
    } else if lower.contains("request exceeds")
        || lower.contains("body exceeds")
        || lower.contains("headers exceed")
    {
        (
            413,
            "request_too_large",
            "The request exceeds the Studio limit",
        )
    } else if lower.contains("not found") || lower.contains("route not found") {
        (404, "not_found", "The requested resource was not found")
    } else if lower.contains("timed out") || lower.contains("timeout") {
        (504, "agent_timeout", "The request timed out")
    } else if lower.contains("invalid") || lower.contains("must not") || lower.contains("exceeds") {
        (400, "invalid_input", "The request is invalid")
    } else if lower.contains("agent") || lower.contains("daemon") || lower.contains("socket") {
        (
            502,
            "agent_unavailable",
            "The local Maestria service is unavailable",
        )
    } else {
        (500, "internal", "Studio could not complete the request")
    }
}

struct HttpRequest {
    method: String,
    path: String,
    headers: std::collections::BTreeMap<String, String>,
    body: Vec<u8>,
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
    let header_end;
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .context("read Studio request")?;
        if read == 0 {
            return Err(anyhow!("connection closed before HTTP headers"));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
        if bytes.len() > MAX_HTTP_BODY {
            return Err(anyhow!("HTTP headers exceed Studio limit"));
        }
    }
    let header_text =
        std::str::from_utf8(&bytes[..header_end]).context("HTTP headers are not UTF-8")?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| anyhow!("missing HTTP request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP method"))?
        .to_owned();
    let path = parts
        .next()
        .ok_or_else(|| anyhow!("missing HTTP path"))?
        .to_owned();
    let mut headers = std::collections::BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let content_length = match headers.get("content-length") {
        Some(value) => value
            .parse::<usize>()
            .context("invalid HTTP content length")?,
        None => 0,
    };
    if content_length > MAX_HTTP_BODY {
        return Err(anyhow!("HTTP body exceeds Studio limit"));
    }
    let mut body = bytes[header_end..].to_vec();
    while body.len() < content_length {
        let read = stream
            .read(&mut chunk)
            .await
            .context("read Studio request body")?;
        if read == 0 {
            return Err(anyhow!("connection closed before HTTP body"));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

async fn write_http(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        422 => "Unprocessable Entity",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'\r\n\
         Referrer-Policy: no-referrer\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .context("write Studio response headers")?;
    stream
        .write_all(body)
        .await
        .context("write Studio response body")
}

fn json_response<T: Serialize>(status: u16, value: &T) -> Result<(u16, &'static str, Vec<u8>)> {
    Ok((
        status,
        "application/json",
        serde_json::to_vec(value).context("encode Studio JSON response")?,
    ))
}
#[cfg(test)]
mod tests {
    use super::{classify_http_error, is_static_frontend_get, origin_allowed};
    use crate::AgentProfile;

    #[test]
    fn default_agent_disables_tools_and_sessions() {
        let profile = AgentProfile::default();
        assert_eq!(profile.args, vec!["--no-tools", "--no-session", "acp"]);
    }

    #[test]
    fn origin_check_requires_same_origin_for_writes() {
        let expected = "http://127.0.0.1:43123";
        assert!(origin_allowed("GET", None, expected));
        assert!(!origin_allowed("POST", None, expected));
        assert!(origin_allowed("POST", Some(&expected.to_owned()), expected));
        assert!(!origin_allowed(
            "POST",
            Some(&"http://localhost:43123".to_owned()),
            expected
        ));
        assert!(!origin_allowed(
            "POST",
            Some(&"https://attacker.invalid".to_owned()),
            expected
        ));
    }

    #[test]
    fn frontend_shell_is_public_but_api_remains_authenticated() {
        assert!(is_static_frontend_get("GET", "/"));
        assert!(is_static_frontend_get("GET", "/app.js"));
        assert!(!is_static_frontend_get("GET", "/api/bootstrap"));
        assert!(!is_static_frontend_get("POST", "/api/notebooks"));
    }

    #[test]
    fn source_selection_denial_has_no_metadata_leak() {
        assert_eq!(
            classify_http_error("daemon request failed: source not selected"),
            (
                403,
                "source_not_selected",
                "Evidence is not selected in this notebook"
            )
        );
    }
}
