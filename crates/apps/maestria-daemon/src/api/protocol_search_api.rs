use std::{collections::BTreeMap, fmt, path::PathBuf};

use anyhow::Result;
use maestria_domain::RealmId;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, net::UnixStream};

use super::{
    ClientErrorCode, DaemonRequestError, EvidenceResponse, FederationCredential,
    RetrievalStatusResponse, SearchResponse,
};

pub const SEARCH_API_PROTOCOL: &str = "sillage.search";
pub const SEARCH_API_VERSION: u16 = 1;
pub const SEARCH_API_VERSION_2: u16 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchApiOperation {
    Search {
        query: String,
        limit: usize,
    },
    /// Bounded lexical passage and filename/path search that supersedes
    /// prior interactive work.
    InteractiveSearch {
        query: String,
        limit: usize,
    },
    Status,
    Evidence {
        evidence_id: u64,
    },
    IndexingStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SearchApiResponse {
    Search(SearchResponse),
    Status(Box<RetrievalStatusResponse>),
    Evidence(EvidenceResponse),
    IndexingStatus(Box<SearchApiIndexingStatusResponse>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchApiIndexingStatusResponse {
    pub approved_root_count: usize,
    pub indexed_file_count: usize,
    pub inventory_truncated: bool,
    /// Fresh, indexed PDFs that require optional OCR before passage search.
    #[serde(default)]
    pub ocr_needed_file_count: usize,
    pub excluded_file_count: usize,
    pub exclusions_by_reason: BTreeMap<String, usize>,
    pub supported_formats: Vec<String>,
    pub ignored_by_default: Vec<String>,
    pub scanning: bool,
    pub pending_file_count: usize,
    pub last_scan_unix_ms: Option<u64>,
    pub last_scan_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SearchApiRequest {
    pub protocol: String,
    pub version: u16,
    pub consumer_realm: RealmId,
    pub credential: FederationCredential,
    pub operation: SearchApiOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct SearchApiReply {
    pub protocol: String,
    pub version: u16,
    pub response: Option<SearchApiResponse>,
    pub error: Option<String>,
    pub error_code: Option<ClientErrorCode>,
}

#[derive(Clone)]
pub struct SearchApiClient {
    socket_path: PathBuf,
    consumer_realm: RealmId,
    credential: FederationCredential,
}

impl fmt::Debug for SearchApiClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchApiClient")
            .field("socket_path", &self.socket_path)
            .field("consumer_realm", &self.consumer_realm)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

impl SearchApiClient {
    /// Builds a typed search-only client for a different process. The
    /// credential is the one-time secret issued by `maestria realm grant`;
    /// this client never reads or carries the provider's instance token.
    pub fn consumer(
        socket_path: PathBuf,
        consumer_realm: RealmId,
        credential: String,
    ) -> Result<Self> {
        Ok(Self {
            socket_path,
            consumer_realm,
            credential: FederationCredential::try_from(credential)?,
        })
    }

    /// Sends one versioned request over the local daemon socket.
    ///
    /// Requests and replies are single capped NDJSON frames.
    ///
    /// # Cancellation
    /// Dropping this future closes the socket and stops waiting for the reply. The daemon cancels
    /// the request handler on disconnect; an already-running blocking evidence read may finish.
    pub async fn request(
        &self,
        operation: SearchApiOperation,
    ) -> std::result::Result<SearchApiResponse, DaemonRequestError> {
        validate_operation(&operation)?;
        let version = match &operation {
            SearchApiOperation::IndexingStatus | SearchApiOperation::InteractiveSearch { .. } => {
                SEARCH_API_VERSION_2
            }
            _ => SEARCH_API_VERSION,
        };
        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|error| {
                request_error(
                    ClientErrorCode::DaemonUnavailable,
                    format!(
                        "connect daemon socket {}: {error}",
                        self.socket_path.display()
                    ),
                )
            })?;
        let request = SearchApiRequest {
            protocol: SEARCH_API_PROTOCOL.to_owned(),
            version,
            consumer_realm: self.consumer_realm.clone(),
            credential: self.credential.clone(),
            operation,
        };
        let mut line = serde_json::to_vec(&request).map_err(|error| {
            request_error(
                ClientErrorCode::InvalidInput,
                format!("encode search API request: {error}"),
            )
        })?;
        line.push(b'\n');
        if line.len() > super::MAX_REQUEST_BYTES {
            return Err(request_error(
                ClientErrorCode::RequestTooLarge,
                "search API request exceeds size limit".to_owned(),
            ));
        }
        stream.write_all(&line).await.map_err(|error| {
            request_error(
                ClientErrorCode::DaemonUnavailable,
                format!("send search API request: {error}"),
            )
        })?;
        let response_line = super::protocol::read_capped_ndjson_line(&mut stream)
            .await
            .map_err(|error| {
                let message = error.to_string();
                let code = if message.contains("exceeds maximum length") {
                    ClientErrorCode::RequestTooLarge
                } else {
                    ClientErrorCode::DaemonUnavailable
                };
                request_error(code, format!("read search API response: {message}"))
            })?;
        let value: serde_json::Value =
            serde_json::from_slice(response_line.trim_ascii()).map_err(|error| {
                request_error(
                    ClientErrorCode::ProtocolVersionMismatch,
                    format!("decode search API response: {error}"),
                )
            })?;
        if value.get("protocol").and_then(serde_json::Value::as_str) != Some(SEARCH_API_PROTOCOL)
            || value.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(version))
        {
            return Err(request_error(
                ClientErrorCode::ProtocolVersionMismatch,
                "daemon does not speak the supported search API protocol version".to_owned(),
            ));
        }
        let reply: SearchApiReply = serde_json::from_value(value).map_err(|error| {
            request_error(
                ClientErrorCode::Internal,
                format!("decode search API reply: {error}"),
            )
        })?;
        match (reply.response, reply.error, reply.error_code) {
            (Some(response), None, None) => Ok(response),
            (Some(_), None, Some(_)) => Err(request_error(
                ClientErrorCode::Internal,
                "search API response contained an error code with a success response".to_owned(),
            )),
            (None, Some(message), code) => Err(request_error(
                match code {
                    Some(code) => code,
                    None => ClientErrorCode::Internal,
                },
                message,
            )),
            _ => Err(request_error(
                ClientErrorCode::Internal,
                "search API response had invalid shape".to_owned(),
            )),
        }
    }
}

pub(super) fn validate_operation(
    operation: &SearchApiOperation,
) -> std::result::Result<(), DaemonRequestError> {
    match operation {
        SearchApiOperation::Search { limit, .. }
            if !(1..=super::protocol::MAX_SEARCH_LIMIT).contains(limit) =>
        {
            Err(request_error(
                ClientErrorCode::InvalidInput,
                format!(
                    "search limit must be between 1 and {}",
                    super::protocol::MAX_SEARCH_LIMIT
                ),
            ))
        }
        SearchApiOperation::InteractiveSearch { query, limit } => {
            if !(1..=super::protocol::MAX_SEARCH_LIMIT).contains(limit) {
                return Err(request_error(
                    ClientErrorCode::InvalidInput,
                    format!(
                        "interactive search limit must be between 1 and {}",
                        super::protocol::MAX_SEARCH_LIMIT
                    ),
                ));
            }
            if query.trim().is_empty()
                || query.len() > maestria_retrieval::INTERACTIVE_MAX_QUERY_BYTES
            {
                return Err(request_error(
                    ClientErrorCode::InvalidInput,
                    format!(
                        "interactive query must contain text and be at most {} bytes",
                        maestria_retrieval::INTERACTIVE_MAX_QUERY_BYTES
                    ),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn reply(
    response: Option<SearchApiResponse>,
    error: Option<String>,
    error_code: Option<ClientErrorCode>,
    version: u16,
) -> SearchApiReply {
    SearchApiReply {
        protocol: SEARCH_API_PROTOCOL.to_owned(),
        version,
        response,
        error,
        error_code,
    }
}

fn request_error(code: ClientErrorCode, message: String) -> DaemonRequestError {
    DaemonRequestError { code, message }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unavailable_socket_has_a_typed_error() -> Result<()> {
        let directory = crate::test_support::TempDir::create()?;
        let socket_path = directory.path().join("daemon.sock");
        let client = SearchApiClient::consumer(
            socket_path,
            RealmId::try_from("a".repeat(64))?,
            "b".repeat(64),
        )?;

        let error = client
            .request(SearchApiOperation::Status)
            .await
            .err()
            .ok_or_else(|| anyhow::anyhow!("absent daemon socket accepted request"))?;

        assert_eq!(error.code, ClientErrorCode::DaemonUnavailable);
        Ok(())
    }
    #[test]
    fn interactive_search_validates_query_bytes_and_result_bound() {
        assert!(
            validate_operation(&SearchApiOperation::InteractiveSearch {
                query: "x".repeat(maestria_retrieval::INTERACTIVE_MAX_QUERY_BYTES),
                limit: super::super::protocol::MAX_SEARCH_LIMIT,
            })
            .is_ok()
        );
        assert!(
            validate_operation(&SearchApiOperation::InteractiveSearch {
                query: "x".repeat(maestria_retrieval::INTERACTIVE_MAX_QUERY_BYTES + 1),
                limit: 1,
            })
            .is_err()
        );
        assert!(
            validate_operation(&SearchApiOperation::InteractiveSearch {
                query: "needle".to_string(),
                limit: super::super::protocol::MAX_SEARCH_LIMIT + 1,
            })
            .is_err()
        );
    }
}
