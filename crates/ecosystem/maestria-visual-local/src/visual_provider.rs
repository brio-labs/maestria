use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use maestria_domain::RepresentationName;
use maestria_ports::{
    EmbeddingIdentity, EmbeddingResponse, PortError, ProviderDisclosure, ProviderEndpoint,
    ProviderTransport, RetentionPolicy, VisualEmbeddingProvider, VisualEmbeddingRequest,
    VisualSource,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const VISUAL_ENDPOINT_PATH: &str = "/v1/embeddings";

/// Local HTTP adapter for any visual model exposing Maestria's vector contract.
///
/// The model runtime is deliberately outside this crate. A small CPU ONNX
/// runtime and a larger Qwen runtime can therefore share the same Rust port.
#[derive(Clone)]
pub struct LocalHttpVisualProvider {
    model: String,
    identity: EmbeddingIdentity,
    transport: Arc<dyn ProviderTransport>,
}

impl LocalHttpVisualProvider {
    /// Creates a local, no-retention provider using the default HTTP transport.
    pub fn new(
        endpoint: &str,
        model: &str,
        identity: EmbeddingIdentity,
    ) -> Result<Self, PortError> {
        let endpoint = ProviderEndpoint::loopback_http(endpoint, VISUAL_ENDPOINT_PATH)?;
        validate_profile(model, &identity)?;
        Ok(Self {
            model: model.to_string(),
            identity,
            transport: Arc::new(UreqTransport::new(endpoint)),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_transport(
        endpoint: &str,
        model: &str,
        identity: EmbeddingIdentity,
        transport: Arc<dyn ProviderTransport>,
    ) -> Result<Self, PortError> {
        let expected_endpoint = ProviderEndpoint::loopback_http(endpoint, VISUAL_ENDPOINT_PATH)?;
        if transport.endpoint() != &expected_endpoint {
            return Err(PortError::InvalidInputContext {
                context: "visual transport endpoint mismatch",
                source: "transport endpoint does not match provider endpoint".to_string(),
            });
        }
        validate_profile(model, &identity)?;
        Ok(Self {
            model: model.to_string(),
            identity,
            transport,
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }
}

fn validate_profile(model: &str, identity: &EmbeddingIdentity) -> Result<(), PortError> {
    if model.trim().is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "visual model is empty",
            source: "model must contain a non-whitespace value".to_string(),
        });
    }
    if identity.representation != RepresentationName::new("visual_page_v1") {
        return Err(PortError::InvalidInputContext {
            context: "visual provider representation is invalid",
            source: "identity representation must be visual_page_v1".to_string(),
        });
    }
    if identity.fingerprint.model != model {
        return Err(PortError::InvalidInputContext {
            context: "visual model identity mismatch",
            source: "model does not match the provider identity".to_string(),
        });
    }
    if identity.fingerprint.dimensions == 0 {
        return Err(PortError::InvalidInputContext {
            context: "visual provider dimensions are zero",
            source: "identity dimensions must be positive".to_string(),
        });
    }
    Ok(())
}

impl VisualEmbeddingProvider for LocalHttpVisualProvider {
    fn disclosure(&self) -> ProviderDisclosure {
        self.transport.disclosure().clone()
    }

    fn embed_query(
        &self,
        query: &str,
        identity: EmbeddingIdentity,
    ) -> Result<EmbeddingResponse, PortError> {
        if query.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "visual query is empty",
                source: "query must contain a non-whitespace value".to_string(),
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
        if request.identity != self.identity {
            return Err(PortError::InvalidInputContext {
                context: "visual request identity mismatch",
                source: "request identity does not match the provider identity".to_string(),
            });
        }
        if request.bytes.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "visual source bytes are empty",
                source: "source bytes must contain data".to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "visual request identity mismatch",
                source: "request identity does not match the provider identity".to_string(),
            });
        }
        let body = serde_json::to_vec(&request).map_err(|error| PortError::InternalContext {
            context: "encode visual request",
            source: error.to_string(),
        })?;
        let response = self.transport.post(body)?;
        let parsed: VisualApiResponse =
            serde_json::from_slice(&response).map_err(|error| PortError::DownstreamContext {
                context: "decode visual response",
                source: error.to_string(),
            })?;
        let first = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "decode visual response data",
                source: "visual response contained no data".to_string(),
            })?;
        let expected = self.identity.fingerprint.dimensions as usize;
        if first.embedding.len() != expected
            || first.embedding.iter().any(|value| !value.is_finite())
        {
            return Err(PortError::DownstreamContext {
                context: "validate visual response vector",
                source: format!("visual response dimensions must be {expected} finite values"),
            });
        }
        Ok(EmbeddingResponse {
            vector: first.embedding,
            provider_id: self.transport.endpoint().as_str().to_string(),
            model: self.model.clone(),
            model_version: if parsed.model.is_empty() {
                self.model.clone()
            } else {
                parsed.model
            },
            identity: self.identity.clone(),
            disclosure: self.transport.disclosure().clone(),
        })
    }
}

#[derive(Debug, Clone)]
struct UreqTransport {
    endpoint: ProviderEndpoint,
    disclosure: ProviderDisclosure,
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new(endpoint: ProviderEndpoint) -> Self {
        Self {
            endpoint,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(30))
                .redirects(0)
                .build(),
        }
    }
}

impl ProviderTransport for UreqTransport {
    fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    fn disclosure(&self) -> &ProviderDisclosure {
        &self.disclosure
    }

    fn post(&self, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.agent
            .post(self.endpoint.as_str())
            .set("content-type", "application/json")
            .send_bytes(&body)
            .map_err(|error| PortError::DownstreamContext {
                context: "visual request failed",
                source: error.to_string(),
            })?
            .into_string()
            .map(String::into_bytes)
            .map_err(|error| PortError::DownstreamContext {
                context: "read visual response",
                source: error.to_string(),
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

#[cfg(test)]
mod tests;
