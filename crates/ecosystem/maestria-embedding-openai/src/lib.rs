use maestria_ports::{
    EmbeddingIdentity, EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse,
    PortError, ProviderDisclosure, RetentionPolicy,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;
#[derive(Clone)]
pub struct LocalHttpEmbeddingProvider {
    endpoint: Url,
    model: String,
    dimensions: Option<usize>,
    identity: EmbeddingIdentity,
    document_template: String,
    query_template: String,
    disclosure: ProviderDisclosure,
    transport: Arc<dyn EmbeddingTransport>,
}

impl LocalHttpEmbeddingProvider {
    pub fn new(endpoint: &str, model: &str, dimensions: Option<usize>) -> Result<Self, PortError> {
        let identity = legacy_identity(model, dimensions)?;
        Self::with_profile(
            endpoint,
            model,
            dimensions,
            identity,
            "{{text}}".to_string(),
            "{{text}}".to_string(),
            ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        )
    }

    pub fn with_profile(
        endpoint: &str,
        model: &str,
        dimensions: Option<usize>,
        identity: EmbeddingIdentity,
        document_template: String,
        query_template: String,
        disclosure: ProviderDisclosure,
    ) -> Result<Self, PortError> {
        let endpoint = parse_loopback_endpoint(endpoint)?;
        if model.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "embedding model must not be empty".to_string(),
            });
        }
        if dimensions == Some(0) {
            return Err(PortError::InvalidInput {
                message: "embedding dimensions must be positive".to_string(),
            });
        }
        if model != identity.fingerprint.model {
            return Err(PortError::InvalidInput {
                message: "embedding model does not match fingerprint".to_string(),
            });
        }
        if dimensions.is_some_and(|value| value != identity.fingerprint.dimensions as usize) {
            return Err(PortError::InvalidInput {
                message: "embedding dimensions do not match fingerprint".to_string(),
            });
        }
        if !document_template.contains("{{text}}") || !query_template.contains("{{text}}") {
            return Err(PortError::InvalidInput {
                message: "embedding templates must contain the {{text}} placeholder".to_string(),
            });
        }
        Ok(Self {
            endpoint,
            model: model.to_string(),
            dimensions,
            identity,
            document_template,
            query_template,
            disclosure,
            transport: Arc::new(UreqTransport::default()),
        })
    }

    pub fn with_transport(
        endpoint: &str,
        model: &str,
        dimensions: Option<usize>,
        transport: Arc<dyn EmbeddingTransport>,
    ) -> Result<Self, PortError> {
        let mut provider = Self::new(endpoint, model, dimensions)?;
        provider.transport = transport;
        Ok(provider)
    }

    pub fn identity(&self) -> &EmbeddingIdentity {
        &self.identity
    }
}

fn legacy_identity(model: &str, dimensions: Option<usize>) -> Result<EmbeddingIdentity, PortError> {
    let dimensions = dimensions.ok_or_else(|| PortError::InvalidInput {
        message: "embedding dimensions are required for generation-aware indexing".to_string(),
    })?;
    EmbeddingIdentity::legacy(model.to_string(), dimensions)
}

impl EmbeddingProvider for LocalHttpEmbeddingProvider {
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        if request.text.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "embedding text must not be empty".to_string(),
            });
        }
        if request.model != self.model {
            return Err(PortError::InvalidInput {
                message: "embedding request model does not match provider".to_string(),
            });
        }
        if request.identity != self.identity {
            return Err(PortError::InvalidInput {
                message: "embedding request identity does not match provider".to_string(),
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
        let body = serde_json::to_vec(&payload).map_err(|error| PortError::Internal {
            message: format!("encode embedding request: {error}"),
        })?;
        let response = self.transport.post(self.endpoint.as_str(), body)?;
        let parsed: EmbeddingApiResponse =
            serde_json::from_slice(&response).map_err(|error| PortError::Downstream {
                message: format!("decode embedding response: {error}"),
            })?;
        let first = parsed
            .data
            .into_iter()
            .next()
            .ok_or_else(|| PortError::Downstream {
                message: "embedding response contained no data".to_string(),
            })?;
        validate_vector(&first.embedding, self.dimensions)?;
        let model_version = if parsed.model.trim().is_empty() {
            self.model.clone()
        } else {
            parsed.model
        };
        Ok(EmbeddingResponse {
            vector: first.embedding,
            provider_id: self.endpoint.to_string(),
            model: self.model.clone(),
            model_version,
            identity: self.identity.clone(),
            disclosure: self.disclosure.clone(),
        })
    }
    fn identity(&self) -> Option<EmbeddingIdentity> {
        Some(self.identity.clone())
    }
}

pub trait EmbeddingTransport: Send + Sync {
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
                .timeout(std::time::Duration::from_secs(15))
                .redirects(0)
                .build(),
        }
    }
}

impl EmbeddingTransport for UreqTransport {
    fn post(&self, endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        let response = self
            .agent
            .post(endpoint)
            .set("content-type", "application/json")
            .send_bytes(&body)
            .map_err(|error| PortError::Downstream {
                message: format!("embedding request failed: {error}"),
            })?;
        response
            .into_string()
            .map(String::into_bytes)
            .map_err(|error| PortError::Downstream {
                message: format!("read embedding response: {error}"),
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
    let url = Url::parse(endpoint).map_err(|error| PortError::InvalidInput {
        message: format!("invalid embedding endpoint: {error}"),
    })?;
    let is_loopback =
        url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"));
    if !is_loopback
        || url.path() != "/v1/embeddings"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(PortError::InvalidInput {
            message: "embedding endpoint must be an http loopback /v1/embeddings URL".to_string(),
        });
    }
    Ok(url)
}

fn validate_vector(vector: &[f32], dimensions: Option<usize>) -> Result<(), PortError> {
    if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
        return Err(PortError::InvalidInput {
            message: "embedding response must contain finite values".to_string(),
        });
    }
    if dimensions.is_some_and(|expected| expected != vector.len()) {
        return Err(PortError::InvalidInput {
            message: "embedding response dimensions do not match configuration".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
