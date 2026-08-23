use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::RealmId;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
};

use super::{
    ClientAuthentication, ClientOperation, ClientRequest, ClientResponse, FederationCredential,
    MAX_SEARCH_LIMIT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientErrorCode {
    Unauthorized,
    InvalidInput,
    NotFound,
    SourceUnavailable,
    SourceNotSelected,
    RevisionConflict,
    NoEvidence,
    RequestTooLarge,
    /// The daemon socket accepted no connection: nothing is serving the
    /// instance right now (never started, stopped, or killed). Clients may
    /// treat this as "run locally" instead of as a failure.
    DaemonUnavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonRequestError {
    pub code: ClientErrorCode,
    pub message: String,
}

impl std::fmt::Display for DaemonRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "daemon request failed ({:?}): {}",
            self.code, self.message
        )
    }
}

impl std::error::Error for DaemonRequestError {}

#[derive(Debug, Deserialize)]
pub(crate) struct ClientReply {
    pub(crate) response: Option<ClientResponse>,
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) error_code: Option<ClientErrorCode>,
}

#[derive(Serialize)]
pub(crate) struct ClientReplyOut {
    pub(crate) response: Option<ClientResponse>,
    pub(crate) error: Option<String>,
    pub(crate) error_code: Option<ClientErrorCode>,
}

#[derive(Debug, Clone)]
pub struct DaemonClient {
    socket_path: PathBuf,
    authentication: ClientAuthentication,
}

impl DaemonClient {
    pub fn from_instance(layout: &InstanceLayout) -> Result<Self> {
        let token_path = crate::api::token_path(layout);
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("read daemon token {}", token_path.display()))?
            .trim()
            .to_string();
        crate::api::validate_token(&token)?;
        Ok(Self {
            socket_path: crate::api::socket_path(layout),
            authentication: ClientAuthentication::InstanceToken { token },
        })
    }

    pub fn new(socket_path: PathBuf, token_path: PathBuf) -> Result<Self> {
        let token = fs::read_to_string(&token_path)
            .with_context(|| format!("read daemon token {}", token_path.display()))?
            .trim()
            .to_string();
        crate::api::validate_token(&token)?;
        Ok(Self {
            socket_path,
            authentication: ClientAuthentication::InstanceToken { token },
        })
    }

    pub(crate) fn federation(
        socket_path: PathBuf,
        consumer_realm: RealmId,
        credential: FederationCredential,
    ) -> Self {
        Self {
            socket_path,
            authentication: ClientAuthentication::FederationGrant {
                consumer_realm,
                credential,
            },
        }
    }

    /// Sends one authenticated request over the daemon socket.
    ///
    /// # Cancellation
    ///
    /// Dropping the future closes the owned socket and stops waiting for the
    /// response; the daemon continues handling only the already-written frame.
    pub async fn request(
        &self,
        operation: ClientOperation,
    ) -> std::result::Result<ClientResponse, DaemonRequestError> {
        let limit = match &operation {
            ClientOperation::Search { limit, .. }
            | ClientOperation::FederationSearch { limit, .. } => Some(*limit),
            _ => None,
        };
        if let Some(limit) = limit
            && !(1..=MAX_SEARCH_LIMIT).contains(&limit)
        {
            return Err(DaemonRequestError {
                code: ClientErrorCode::InvalidInput,
                message: format!("search limit must be between 1 and {MAX_SEARCH_LIMIT}"),
            });
        }
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|error| DaemonRequestError {
                code: ClientErrorCode::DaemonUnavailable,
                message: format!(
                    "connect daemon socket {}: {error}",
                    self.socket_path.display()
                ),
            })?;
        let request = ClientRequest {
            authentication: self.authentication.clone(),
            operation,
        };
        let mut line = serde_json::to_vec(&request).map_err(|error| DaemonRequestError {
            code: ClientErrorCode::InvalidInput,
            message: format!("encode daemon request: {error}"),
        })?;
        line.push(b'\n');
        if line.len() > crate::api::MAX_REQUEST_BYTES {
            return Err(DaemonRequestError {
                code: ClientErrorCode::RequestTooLarge,
                message: "daemon request exceeds size limit".to_owned(),
            });
        }
        stream
            .write_all(&line)
            .await
            .map_err(|error| DaemonRequestError {
                code: ClientErrorCode::Internal,
                message: format!("send daemon request: {error}"),
            })?;
        let response_line = read_capped_ndjson_line(&mut stream)
            .await
            .map_err(|error| DaemonRequestError {
                code: ClientErrorCode::Internal,
                message: format!("read daemon response: {error}"),
            })?;
        let reply: ClientReply =
            serde_json::from_slice(response_line.trim_ascii()).map_err(|error| {
                DaemonRequestError {
                    code: ClientErrorCode::Internal,
                    message: format!("decode daemon response: {error}"),
                }
            })?;
        match (reply.response, reply.error, reply.error_code) {
            (Some(response), None, None) => Ok(response),
            (Some(_), None, Some(_)) => Err(DaemonRequestError {
                code: ClientErrorCode::Internal,
                message: "daemon response contained an error code with a success response"
                    .to_owned(),
            }),
            (None, Some(message), code) => Err(DaemonRequestError {
                code: match code {
                    Some(code) => code,
                    None => ClientErrorCode::Internal,
                },
                message,
            }),
            _ => Err(DaemonRequestError {
                code: ClientErrorCode::Internal,
                message: "daemon response had invalid shape".to_owned(),
            }),
        }
    }
}

/// Reads one newline-delimited message without extending the allocation past
/// the protocol cap. Unterminated and oversized messages fail identically.
pub(crate) async fn read_capped_ndjson_line<R>(stream: &mut R) -> Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read_exact(&mut byte).await {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(anyhow!("connection closed before end of message"));
            }
            Err(error) => return Err(error).context("read NDJSON message"),
        }
        if byte[0] == b'\n' {
            return Ok(line);
        }
        if line.len() >= crate::api::MAX_REQUEST_BYTES {
            return Err(anyhow!("NDJSON line exceeds maximum length"));
        }
        line.push(byte[0]);
    }
}
