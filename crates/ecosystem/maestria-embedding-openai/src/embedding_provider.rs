use maestria_ports::{
    EmbeddingIdentity, EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse,
    PortError, ProviderDisclosure, ProviderEndpoint, ProviderTransport, RetentionPolicy,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

const EMBEDDING_ENDPOINT_PATH: &str = "/v1/embeddings";

#[derive(Clone)]
pub struct LocalHttpEmbeddingProvider {
    model: String,
    dimensions: Option<usize>,
    identity: EmbeddingIdentity,
    document_template: String,
    query_template: String,
    transport: Arc<dyn ProviderTransport>,
}

impl LocalHttpEmbeddingProvider {
    pub fn with_profile(
        endpoint: &str,
        model: &str,
        dimensions: Option<usize>,
        identity: EmbeddingIdentity,
        document_template: String,
        query_template: String,
    ) -> Result<Self, PortError> {
        let endpoint = ProviderEndpoint::loopback_http(endpoint, EMBEDDING_ENDPOINT_PATH)?;
        validate_profile(
            model,
            dimensions,
            &identity,
            &document_template,
            &query_template,
        )?;
        Ok(Self {
            model: model.to_string(),
            dimensions,
            identity,
            document_template,
            query_template,
            transport: Arc::new(UreqTransport::new(endpoint)),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_transport(
        endpoint: &str,
        model: &str,
        dimensions: Option<usize>,
        identity: EmbeddingIdentity,
        transport: Arc<dyn ProviderTransport>,
    ) -> Result<Self, PortError> {
        let expected_endpoint = ProviderEndpoint::loopback_http(endpoint, EMBEDDING_ENDPOINT_PATH)?;
        if transport.endpoint() != &expected_endpoint {
            return Err(PortError::InvalidInputContext {
                context: "embedding transport endpoint mismatch",
                source: "transport endpoint does not match provider endpoint".to_string(),
            });
        }
        validate_profile(model, dimensions, &identity, "{{text}}", "{{text}}")?;
        Ok(Self {
            model: model.to_string(),
            dimensions,
            identity,
            document_template: "{{text}}".to_string(),
            query_template: "{{text}}".to_string(),
            transport,
        })
    }

    pub fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }
}

fn validate_profile(
    model: &str,
    dimensions: Option<usize>,
    identity: &EmbeddingIdentity,
    document_template: &str,
    query_template: &str,
) -> Result<(), PortError> {
    if model.trim().is_empty() {
        return Err(PortError::InvalidInputContext {
            context: "embedding model is empty",
            source: "model must contain a non-whitespace value".to_string(),
        });
    }
    if dimensions == Some(0) {
        return Err(PortError::InvalidInputContext {
            context: "embedding dimensions are zero",
            source: "dimensions must be positive when provided".to_string(),
        });
    }
    if model != identity.fingerprint.model.as_str() {
        return Err(PortError::InvalidInputContext {
            context: "embedding model identity mismatch",
            source: "model does not match the identity fingerprint".to_string(),
        });
    }
    if dimensions.is_some_and(|value| value != identity.fingerprint.dimensions as usize) {
        return Err(PortError::InvalidInputContext {
            context: "embedding dimensions identity mismatch",
            source: "dimensions do not match the identity fingerprint".to_string(),
        });
    }
    if !document_template.contains("{{text}}") || !query_template.contains("{{text}}") {
        return Err(PortError::InvalidInputContext {
            context: "embedding template is missing text placeholder",
            source: "document and query templates must contain {{text}}".to_string(),
        });
    }
    Ok(())
}

impl EmbeddingProvider for LocalHttpEmbeddingProvider {
    fn disclosure(&self) -> ProviderDisclosure {
        self.transport.disclosure().clone()
    }
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        if request.text.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "embedding text is empty",
                source: "text must contain a non-whitespace value".to_string(),
            });
        }
        if request.model != self.model {
            return Err(PortError::InvalidInputContext {
                context: "embedding request model mismatch",
                source: "request model does not match the provider model".to_string(),
            });
        }
        if request.identity != self.identity {
            return Err(PortError::InvalidInputContext {
                context: "embedding request identity mismatch",
                source: "request identity does not match the provider identity".to_string(),
            });
        }
        let template = match request.kind {
            EmbeddingInputKind::Document => &self.document_template,
            EmbeddingInputKind::Query => &self.query_template,
        };
        let input = template.replace("{{text}}", &request.text);
        let payload = EmbeddingPayload {
            input,
            model: self.model.clone(),
            dimensions: self.dimensions,
        };
        let body = serde_json::to_vec(&payload).map_err(|error| PortError::InternalContext {
            context: "encode embedding request",
            source: error.to_string(),
        })?;
        let response = self.transport.post(body)?;
        let parsed: EmbeddingApiResponse =
            serde_json::from_slice(&response).map_err(|error| PortError::DownstreamContext {
                context: "decode embedding response",
                source: error.to_string(),
            })?;
        let first = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| PortError::DownstreamContext {
                context: "decode embedding response data",
                source: "embedding response contained no data".to_string(),
            })?;
        validate_vector(&first.embedding, self.dimensions)?;
        let model_version = if parsed.model.trim().is_empty() {
            self.model.clone()
        } else {
            parsed.model
        };
        Ok(EmbeddingResponse {
            vector: first.embedding,
            provider_id: self.transport.endpoint().as_str().to_string(),
            model: self.model.clone(),
            model_version,
            identity: self.identity.clone(),
            disclosure: self.transport.disclosure().clone(),
        })
    }
    fn identity(&self) -> Option<EmbeddingIdentity> {
        Some(self.identity.clone())
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
                .timeout(std::time::Duration::from_secs(15))
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
                context: "embedding request failed",
                source: error.to_string(),
            })?
            .into_string()
            .map(String::into_bytes)
            .map_err(|error| PortError::DownstreamContext {
                context: "read embedding response",
                source: error.to_string(),
            })
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingPayload {
    input: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingApiResponse {
    data: Vec<EmbeddingData>,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

pub fn parse_loopback_endpoint(endpoint: &str) -> Result<Url, PortError> {
    ProviderEndpoint::loopback_http(endpoint, EMBEDDING_ENDPOINT_PATH).and_then(|endpoint| {
        Url::parse(endpoint.as_str()).map_err(|error| PortError::InvalidInputContext {
            context: "invalid provider endpoint",
            source: error.to_string(),
        })
    })
}

fn validate_vector(vector: &[f32], dimensions: Option<usize>) -> Result<(), PortError> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(PortError::InvalidInputContext {
            context: "embedding response vector is invalid",
            source: "response must contain finite values".to_string(),
        });
    }
    if dimensions.is_some_and(|expected| expected != vector.len()) {
        return Err(PortError::InvalidInputContext {
            context: "embedding response dimensions mismatch",
            source: "response vector dimensions do not match configuration".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
