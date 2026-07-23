#![forbid(unsafe_code)]

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use maestria_domain::RepresentationName;
use maestria_ports::{
    EmbeddingIdentity, EmbeddingResponse, PortError, ProviderDisclosure, RetentionPolicy,
    VisualEmbeddingProvider, VisualEmbeddingRequest, VisualSource,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

/// Local HTTP adapter for any visual model exposing Maestria's vector contract.
///
/// The model runtime is deliberately outside this crate. A small CPU ONNX
/// runtime and a larger Qwen runtime can therefore share the same Rust port.
#[derive(Clone)]
pub struct LocalHttpVisualProvider {
    endpoint: Url,
    model: String,
    identity: EmbeddingIdentity,
    disclosure: ProviderDisclosure,
    transport: Arc<dyn VisualTransport>,
}

impl LocalHttpVisualProvider {
    /// Creates a local, no-retention provider using the default HTTP transport.
    pub fn new(
        endpoint: &str,
        model: &str,
        identity: EmbeddingIdentity,
    ) -> Result<Self, PortError> {
        Self::with_transport(
            endpoint,
            model,
            identity,
            Arc::new(UreqTransport::default()),
        )
    }

    /// Creates a provider with an injectable transport for deterministic tests.
    pub fn with_transport(
        endpoint: &str,
        model: &str,
        identity: EmbeddingIdentity,
        transport: Arc<dyn VisualTransport>,
    ) -> Result<Self, PortError> {
        let endpoint = parse_loopback_endpoint(endpoint)?;
        if model.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "visual model must not be empty".to_string(),
            });
        }
        if identity.representation != RepresentationName::new("visual_page_v1") {
            return Err(PortError::InvalidInput {
                message: "visual provider identity must use visual_page_v1".to_string(),
            });
        }
        if identity.fingerprint.model != model {
            return Err(PortError::InvalidInput {
                message: "visual model does not match provider identity".to_string(),
            });
        }
        if identity.fingerprint.dimensions == 0 {
            return Err(PortError::InvalidInput {
                message: "visual provider dimensions must be positive".to_string(),
            });
        }
        Ok(Self {
            endpoint,
            model: model.to_string(),
            identity,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
            transport,
        })
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

impl VisualEmbeddingProvider for LocalHttpVisualProvider {
    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(self.disclosure.clone())
    }

    fn embed_query(
        &self,
        query: &str,
        identity: EmbeddingIdentity,
    ) -> Result<EmbeddingResponse, PortError> {
        if query.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "visual query must not be empty".to_string(),
            });
        }
        self.embed(VisualEmbeddingPayload {
            model: self.model.clone(),
            input: VisualInput::Text(query.to_string()),
            identity,
        })
    }

    fn embed_source(
        &self,
        request: VisualEmbeddingRequest,
    ) -> Result<EmbeddingResponse, PortError> {
        if request.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "visual source bytes must not be empty".to_string(),
            });
        }
        self.embed(VisualEmbeddingPayload {
            model: self.model.clone(),
            input: VisualInput::Source {
                source: source_payload(&request.source),
                bytes: format!(
                    "data:application/octet-stream;base64,{}",
                    BASE64.encode(request.bytes)
                ),
            },
            identity: request.identity,
        })
    }

    fn identity(&self) -> Option<EmbeddingIdentity> {
        Some(self.identity.clone())
    }
}

impl LocalHttpVisualProvider {
    fn embed(&self, request: VisualEmbeddingPayload) -> Result<EmbeddingResponse, PortError> {
        if request.identity != self.identity {
            return Err(PortError::InvalidInput {
                message: "visual request identity does not match provider".to_string(),
            });
        }
        let body = serde_json::to_vec(&request).map_err(|error| PortError::Internal {
            message: format!("encode visual request: {error}"),
        })?;
        let response = self.transport.post(self.endpoint.as_str(), body)?;
        let parsed: VisualApiResponse =
            serde_json::from_slice(&response).map_err(|error| PortError::Downstream {
                message: format!("decode visual response: {error}"),
            })?;
        let first = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| PortError::Downstream {
                message: "visual response contained no data".to_string(),
            })?;
        let expected = self.identity.fingerprint.dimensions as usize;
        if first.embedding.len() != expected
            || first.embedding.iter().any(|value| !value.is_finite())
        {
            return Err(PortError::Downstream {
                message: format!("visual response dimensions must be {expected} finite values"),
            });
        }
        Ok(EmbeddingResponse {
            vector: first.embedding,
            provider_id: self.endpoint.to_string(),
            model: self.model.clone(),
            model_version: if parsed.model.is_empty() {
                self.model.clone()
            } else {
                parsed.model
            },
            identity: self.identity.clone(),
            disclosure: self.disclosure.clone(),
        })
    }
}

/// Transport boundary kept separate from the model protocol for unit testing.
pub trait VisualTransport: Send + Sync {
    fn post(&self, endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError>;
}

#[derive(Debug, Clone)]
struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .redirects(0)
                .build(),
        }
    }
}

impl VisualTransport for UreqTransport {
    fn post(&self, endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.agent
            .post(endpoint)
            .set("content-type", "application/json")
            .send_bytes(&body)
            .map_err(|error| PortError::Downstream {
                message: format!("visual request failed: {error}"),
            })?
            .into_string()
            .map(String::into_bytes)
            .map_err(|error| PortError::Downstream {
                message: format!("read visual response: {error}"),
            })
    }
}

#[derive(Debug, Serialize)]
struct VisualEmbeddingPayload {
    model: String,
    input: VisualInput,
    #[serde(skip)]
    identity: EmbeddingIdentity,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum VisualInput {
    Text(String),
    Source {
        source: VisualSourcePayload,
        bytes: String,
    },
}

#[derive(Debug, Serialize)]
struct VisualSourcePayload {
    kind: &'static str,
    blob: String,
    page_start: Option<u32>,
    page_end: Option<u32>,
    page: Option<u32>,
    x: Option<u32>,
    y: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VisualApiResponse {
    data: Vec<VisualData>,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct VisualData {
    embedding: Vec<f32>,
}

fn source_payload(source: &VisualSource) -> VisualSourcePayload {
    match source {
        VisualSource::Page {
            blob,
            page_start,
            page_end,
        } => VisualSourcePayload {
            kind: "page",
            blob: blob.to_string(),
            page_start: Some(*page_start),
            page_end: Some(*page_end),
            page: None,
            x: None,
            y: None,
            width: None,
            height: None,
        },
        VisualSource::Region {
            blob,
            page,
            x,
            y,
            width,
            height,
        } => VisualSourcePayload {
            kind: "region",
            blob: blob.to_string(),
            page_start: None,
            page_end: None,
            page: Some(*page),
            x: Some(*x),
            y: Some(*y),
            width: Some(*width),
            height: Some(*height),
        },
    }
}

fn parse_loopback_endpoint(endpoint: &str) -> Result<Url, PortError> {
    let url = Url::parse(endpoint).map_err(|error| PortError::InvalidInput {
        message: format!("invalid visual endpoint: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/embeddings"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(PortError::InvalidInput {
            message: "visual endpoint must be an http loopback /v1/embeddings URL".to_string(),
        });
    }
    Ok(url)
}

#[cfg(test)]
mod tests;
