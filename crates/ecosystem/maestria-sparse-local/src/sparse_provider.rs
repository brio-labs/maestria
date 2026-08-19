use maestria_domain::RepresentationName;
use maestria_ports::{
    DEFAULT_MAX_SPARSE_TERMS, LearnedSparseProvider, PortError, ProviderDisclosure,
    ProviderEndpoint, ProviderTransport, SPARSE_REPRESENTATION_V1, SparseIdentity, SparseInputKind,
    SparseTermWeight, SparseVector,
};
use std::sync::Arc;

use crate::dto::{
    SparseApiResponse, SparseBatchApiResponse, SparseEncodeBatchPayload, SparseEncodePayload,
    SparseKindWire,
};

const SPARSE_ENDPOINT_PATH: &str = "/v1/sparse";

/// Local HTTP adapter for any SPLADE-family model exposing Maestria's sparse
/// vector contract.
///
/// The model runtime is deliberately outside this crate. The sidecar applies
/// the query/document templates and returns deduplicated, ascending term ids
/// with positive weights bounded by the configured term cap.
#[derive(Clone)]
pub struct LocalHttpSparseProvider {
    model: String,
    identity: SparseIdentity,
    transport: Arc<dyn ProviderTransport>,
}

impl LocalHttpSparseProvider {
    /// Creates a local, no-retention provider using the default HTTP transport.
    pub fn new(endpoint: &str, model: &str, identity: SparseIdentity) -> Result<Self, PortError> {
        let endpoint = ProviderEndpoint::loopback_http(endpoint, SPARSE_ENDPOINT_PATH)?;
        validate_profile(model, &identity)?;
        Ok(Self {
            model: model.to_string(),
            identity,
            transport: Arc::new(maestria_adapter_http::UreqJsonClient::with_batch_timeout(
                endpoint,
                std::time::Duration::from_secs(5),
                std::time::Duration::from_secs(120),
            )),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_transport(
        endpoint: &str,
        model: &str,
        identity: SparseIdentity,
        transport: Arc<dyn ProviderTransport>,
    ) -> Result<Self, PortError> {
        let expected_endpoint = ProviderEndpoint::loopback_http(endpoint, SPARSE_ENDPOINT_PATH)?;
        if transport.endpoint() != &expected_endpoint {
            return Err(PortError::InvalidInputContext {
                context: "sparse transport endpoint mismatch",
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

fn validate_profile(model: &str, identity: &SparseIdentity) -> Result<(), PortError> {
    maestria_ports::validate_model_label(model, "sparse")?;
    if identity.representation != RepresentationName::new(SPARSE_REPRESENTATION_V1) {
        return Err(PortError::InvalidInputContext {
            context: "sparse provider representation is invalid",
            source: "identity representation must be sparse_text_v1".to_string(),
        });
    }
    maestria_adapter_http::validate_model_identity(
        model,
        identity.fingerprint.model.as_str(),
        "sparse",
    )?;
    if identity.fingerprint.vocabulary_size == 0 {
        return Err(PortError::InvalidInputContext {
            context: "sparse provider vocabulary is zero",
            source: "identity vocabulary size must be positive".to_string(),
        });
    }
    identity.validate()
}

impl LearnedSparseProvider for LocalHttpSparseProvider {
    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(self.transport.disclosure().clone())
    }

    fn identity(&self) -> Option<SparseIdentity> {
        Some(self.identity.clone())
    }

    fn encode(
        &self,
        text: &str,
        kind: SparseInputKind,
        identity: SparseIdentity,
    ) -> Result<SparseVector, PortError> {
        if identity != self.identity {
            return Err(PortError::InvalidInputContext {
                context: "sparse provider identity mismatch",
                source: "request identity differs from provider".to_string(),
            });
        }
        if text.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "sparse input text is empty",
                source: "text must contain a non-whitespace token".to_string(),
            });
        }
        let payload = SparseEncodePayload {
            text: text.to_string(),
            kind: match kind {
                SparseInputKind::Query => SparseKindWire::Query,
                SparseInputKind::Document => SparseKindWire::Document,
            },
        };
        let body = serde_json::to_vec(&payload)
            .map_err(|error| PortError::internal("encode sparse request", error.to_string()))?;
        let response = self.transport.post(body)?;
        let parsed: SparseApiResponse = serde_json::from_slice(&response)
            .map_err(|error| PortError::downstream("decode sparse response", error.to_string()))?;
        self.build_vector(parsed, identity)
    }

    fn encode_batch(
        &self,
        texts: &[String],
        kind: SparseInputKind,
        identity: SparseIdentity,
    ) -> Result<Vec<SparseVector>, PortError> {
        if identity != self.identity {
            return Err(PortError::InvalidInputContext {
                context: "sparse provider identity mismatch",
                source: "request identity differs from provider".to_string(),
            });
        }
        if texts.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "sparse batch input is empty",
                source: "texts must contain at least one entry".to_string(),
            });
        }
        if texts.iter().any(|text| text.trim().is_empty()) {
            return Err(PortError::InvalidInputContext {
                context: "sparse input text is empty",
                source: "text must contain a non-whitespace token".to_string(),
            });
        }
        let payload = SparseEncodeBatchPayload {
            texts,
            kind: match kind {
                SparseInputKind::Query => SparseKindWire::Query,
                SparseInputKind::Document => SparseKindWire::Document,
            },
        };
        let body = serde_json::to_vec(&payload).map_err(|error| {
            PortError::internal("encode sparse batch request", error.to_string())
        })?;
        let response = self.transport.post_to("/batch", body)?;
        let parsed: SparseBatchApiResponse =
            serde_json::from_slice(&response).map_err(|error| {
                PortError::downstream("decode sparse batch response", error.to_string())
            })?;
        if parsed.vectors.len() != texts.len() {
            return Err(PortError::DownstreamContext {
                context: "validate sparse batch response",
                source: "response vector count does not match the request".to_string(),
            });
        }
        parsed
            .vectors
            .into_iter()
            .map(|vector| self.build_vector(vector, identity.clone()))
            .collect()
    }
}

impl LocalHttpSparseProvider {
    fn build_vector(
        &self,
        parsed: SparseApiResponse,
        identity: SparseIdentity,
    ) -> Result<SparseVector, PortError> {
        let max_terms = usize::try_from(identity.fingerprint.max_terms).map_err(|_| {
            PortError::InvalidInputContext {
                context: "sparse max_terms exceeds platform range",
                source: "max_terms does not fit this platform".to_string(),
            }
        })?;
        if parsed.term_ids.len() != parsed.weights.len() {
            return Err(PortError::DownstreamContext {
                context: "validate sparse response vector",
                source: "term_ids and weights must have equal length".to_string(),
            });
        }
        if parsed.term_ids.is_empty() {
            return Err(PortError::DownstreamContext {
                context: "validate sparse response vector",
                source: "sparse response contained no terms".to_string(),
            });
        }
        if parsed.term_ids.len() > max_terms || parsed.term_ids.len() > DEFAULT_MAX_SPARSE_TERMS {
            return Err(PortError::DownstreamContext {
                context: "validate sparse response vector",
                source: format!(
                    "sparse response term count must be within {} terms",
                    max_terms.min(DEFAULT_MAX_SPARSE_TERMS)
                ),
            });
        }
        let mut terms = Vec::with_capacity(parsed.term_ids.len());
        let mut previous = None;
        for (term_id, weight) in parsed.term_ids.iter().zip(parsed.weights.iter()) {
            let term_id = *term_id;
            if previous.is_some_and(|previous| term_id <= previous) {
                return Err(PortError::DownstreamContext {
                    context: "validate sparse response vector",
                    source: "term_ids must be strictly ascending and unique".to_string(),
                });
            }
            previous = Some(term_id);
            if term_id >= identity.fingerprint.vocabulary_size {
                return Err(PortError::DownstreamContext {
                    context: "validate sparse response vector",
                    source: "term identifier is outside the vocabulary".to_string(),
                });
            }
            terms.push(SparseTermWeight::new(term_id, *weight).map_err(|_| {
                PortError::DownstreamContext {
                    context: "validate sparse response vector",
                    source: "term weights must be finite and positive".to_string(),
                }
            })?);
        }
        SparseVector::new(identity, terms)
    }
}

#[cfg(test)]
mod tests;
