use maestria_ports::{
    EmbeddingIdentity, EmbeddingInputKind, EmbeddingProvider, EmbeddingRequest, EmbeddingResponse,
    PortError, ProviderDisclosure, ProviderEndpoint, ProviderTransport,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;

const EMBEDDING_ENDPOINT_PATH: &str = "/v1/embeddings";

/// Safety bound on embedding input length in bytes.
///
/// Local serving stacks reject inputs longer than the model context
/// window instead of truncating. This bound keeps requests under an
/// 8k-token context window for any text encoding at two or more bytes
/// per token — the ratio observed on multilingual byte-pair
/// tokenizers (JSON ~2.5 bytes/token, CJK ~7.5 bytes/token) — and
/// never splits a character boundary.
const MAX_INPUT_BYTES: usize = 16_384;

fn truncate_text(text: &str) -> &str {
    maestria_ports::truncate_at_char_boundary(text, MAX_INPUT_BYTES)
}

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
            transport: Arc::new(maestria_adapter_http::UreqJsonClient::new(
                endpoint,
                std::time::Duration::from_secs(15),
            )),
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
    maestria_ports::validate_model_label(model, "embedding")?;
    if dimensions == Some(0) {
        return Err(PortError::InvalidInputContext {
            context: "embedding dimensions are zero",
            source: "dimensions must be positive when provided".to_string(),
        });
    }
    maestria_adapter_http::validate_model_identity(
        model,
        identity.fingerprint.model.as_str(),
        "embedding",
    )?;
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
        let input = template.replace("{{text}}", truncate_text(&request.text));
        let payload = EmbeddingPayload {
            input,
            model: self.model.clone(),
            dimensions: self.dimensions,
        };
        let body = serde_json::to_vec(&payload)
            .map_err(|error| PortError::internal("encode embedding request", error.to_string()))?;
        let response = self.transport.post(body)?;
        let parsed: EmbeddingApiResponse = serde_json::from_slice(&response).map_err(|error| {
            PortError::downstream("decode embedding response", error.to_string())
        })?;
        let first = parsed.data.into_iter().next().ok_or_else(|| {
            PortError::downstream(
                "decode embedding response data",
                "embedding response contained no data",
            )
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

    /// One round-trip for the whole batch: `input` carries every template-
    /// applied text and the response entries are re-ordered by their
    /// `index` field to stay position-aligned with `requests`.
    fn embed_batch(
        &self,
        requests: &[EmbeddingRequest],
    ) -> Result<Vec<EmbeddingResponse>, PortError> {
        if requests.is_empty() {
            return Ok(Vec::new());
        }
        let mut inputs = Vec::with_capacity(requests.len());
        for request in requests {
            if request.text.trim().is_empty() {
                return Err(PortError::InvalidInputContext {
                    context: "embedding text is empty",
                    source: "text must contain a non-whitespace value".to_string(),
                });
            }
            if request.model != self.model || request.identity != self.identity {
                return Err(PortError::InvalidInputContext {
                    context: "embedding request model or identity mismatch",
                    source: "batch requests must match the provider identity".to_string(),
                });
            }
            let template = match request.kind {
                EmbeddingInputKind::Document => &self.document_template,
                EmbeddingInputKind::Query => &self.query_template,
            };
            inputs.push(template.replace("{{text}}", truncate_text(&request.text)));
        }
        let payload = EmbeddingBatchPayload {
            input: inputs,
            model: self.model.clone(),
            dimensions: self.dimensions,
        };
        let body = serde_json::to_vec(&payload)
            .map_err(|error| PortError::internal("encode embedding batch", error.to_string()))?;
        let response = self.transport.post(body)?;
        let mut parsed: EmbeddingApiResponse =
            serde_json::from_slice(&response).map_err(|error| {
                PortError::downstream("decode embedding response", error.to_string())
            })?;
        if parsed.data.len() != requests.len() {
            return Err(PortError::downstream(
                "decode embedding response data",
                format!(
                    "embedding batch returned {} entries for {} inputs",
                    parsed.data.len(),
                    requests.len()
                ),
            ));
        }
        parsed.data.sort_by_key(|entry| entry.index);
        let model_version = if parsed.model.trim().is_empty() {
            self.model.clone()
        } else {
            parsed.model
        };
        parsed
            .data
            .into_iter()
            .zip(requests.iter())
            .map(|(entry, _)| {
                validate_vector(&entry.embedding, self.dimensions)?;
                Ok(EmbeddingResponse {
                    vector: entry.embedding,
                    provider_id: self.transport.endpoint().as_str().to_string(),
                    model: self.model.clone(),
                    model_version: model_version.clone(),
                    identity: self.identity.clone(),
                    disclosure: self.transport.disclosure().clone(),
                })
            })
            .collect::<Result<Vec<_>, PortError>>()
    }
    fn identity(&self) -> Option<EmbeddingIdentity> {
        Some(self.identity.clone())
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingPayload {
    input: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Debug, Serialize)]
struct EmbeddingBatchPayload {
    input: Vec<String>,
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
    #[serde(default)]
    index: usize,
    embedding: Vec<f32>,
}

pub fn parse_loopback_endpoint(endpoint: &str) -> Result<Url, PortError> {
    ProviderEndpoint::loopback_http(endpoint, EMBEDDING_ENDPOINT_PATH).and_then(|endpoint| {
        Url::parse(endpoint.as_str()).map_err(|error| {
            PortError::invalid_input("invalid provider endpoint", error.to_string())
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
