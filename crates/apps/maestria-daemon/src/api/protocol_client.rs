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

#[derive(Debug, Deserialize)]
pub(crate) struct ClientReply {
    pub(crate) response: Option<ClientResponse>,
    pub(crate) error: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ClientReplyOut {
    pub(crate) response: Option<ClientResponse>,
    pub(crate) error: Option<String>,
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

    /// Send an operation to the daemon and return its response.
    ///
    /// # Cancellation
    /// Dropping the future closes the socket and abandons the request. No server-side
    /// cancellation is guaranteed.
    pub async fn request(&self, operation: ClientOperation) -> Result<ClientResponse> {
        let limit = match &operation {
            ClientOperation::Search { limit, .. }
            | ClientOperation::FederationSearch { limit, .. } => Some(*limit),
            _ => None,
        };
        if let Some(limit) = limit
            && !(1..=MAX_SEARCH_LIMIT).contains(&limit)
        {
            return Err(anyhow!(
                "search limit must be between 1 and {MAX_SEARCH_LIMIT}"
            ));
        }
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .with_context(|| format!("connect daemon socket {}", self.socket_path.display()))?;
        let request = ClientRequest {
            authentication: self.authentication.clone(),
            operation,
        };
        let mut line = serde_json::to_vec(&request).context("encode daemon request")?;
        line.push(b'\n');
        if line.len() > crate::api::MAX_REQUEST_BYTES {
            return Err(anyhow!("daemon request exceeds size limit"));
        }
        stream
            .write_all(&line)
            .await
            .context("send daemon request")?;
        let response_line = read_capped_ndjson_line(&mut stream)
            .await
            .context("read daemon response")?;
        let reply: ClientReply =
            serde_json::from_slice(response_line.trim_ascii()).context("decode daemon response")?;
        match (reply.response, reply.error) {
            (Some(response), None) => Ok(response),
            (None, Some(error)) => Err(anyhow!("daemon request rejected: {error}")),
            _ => Err(anyhow!("daemon response had invalid shape")),
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
