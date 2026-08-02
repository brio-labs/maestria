use super::*;
use maestria_ports::contract_tests::fixture_embedding_identity;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
const ENDPOINT: &str = "http://127.0.0.1:8080/v1/embeddings";
fn endpoint() -> Result<ProviderEndpoint, PortError> {
    ProviderEndpoint::loopback_http(ENDPOINT, EMBEDDING_ENDPOINT_PATH)
}

struct FixtureTransport {
    response: Mutex<Option<Result<Vec<u8>, PortError>>>,
    endpoint: ProviderEndpoint,
    disclosure: ProviderDisclosure,
}

impl FixtureTransport {
    fn new(response: Result<Vec<u8>, PortError>) -> Result<Self, PortError> {
        Ok(Self {
            response: Mutex::new(Some(response)),
            endpoint: endpoint()?,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        })
    }
}

impl ProviderTransport for FixtureTransport {
    fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    fn disclosure(&self) -> &ProviderDisclosure {
        &self.disclosure
    }

    fn post(&self, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.response
            .lock()
            .map_err(|_| PortError::Internal {
                message: "fixture lock poisoned".to_string(),
            })?
            .take()
            .ok_or_else(|| PortError::Internal {
                message: "fixture already consumed".to_string(),
            })?
    }
}

struct RecordingTransport {
    response: Vec<u8>,
    body: Mutex<Option<Vec<u8>>>,
    post_count: AtomicUsize,
    endpoint: ProviderEndpoint,
    disclosure: ProviderDisclosure,
}
impl RecordingTransport {
    fn new(response: Vec<u8>, disclosure: ProviderDisclosure) -> Result<Self, PortError> {
        Ok(Self {
            response,
            body: Mutex::new(None),
            post_count: AtomicUsize::new(0),
            endpoint: endpoint()?,
            disclosure,
        })
    }
}

impl ProviderTransport for RecordingTransport {
    fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    fn disclosure(&self) -> &ProviderDisclosure {
        &self.disclosure
    }

    fn post(&self, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.post_count.fetch_add(1, Ordering::Relaxed);
        *self.body.lock().map_err(|_| PortError::Internal {
            message: "recording lock poisoned".to_string(),
        })? = Some(body);
        Ok(self.response.clone())
    }
}
#[test]
fn applies_kind_template_and_preserves_disclosure() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport::new(
        br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ProviderDisclosure {
            remote: true,
            retention: RetentionPolicy::ProviderDefined,
        },
    )?);
    let identity = fixture_embedding_identity("profiled-model", 2)?;
    let mut provider = LocalHttpEmbeddingProvider::with_profile(
        ENDPOINT,
        "profiled-model",
        Some(2),
        identity.clone(),
        "document: {{text}}".to_string(),
        "query: {{text}}".to_string(),
    )?;
    provider.transport = transport.clone();
    let response = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "profiled-model".to_string(),
        kind: EmbeddingInputKind::Query,
        identity,
    })?;
    let body = transport
        .body
        .lock()
        .map_err(|_| PortError::Internal {
            message: "recording lock poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| PortError::Internal {
            message: "recording body missing".to_string(),
        })?;
    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| PortError::Internal {
            message: format!("decode recording body: {error}"),
        })?;
    assert_eq!(payload["input"], "query: hello");
    assert!(response.disclosure.remote);
    assert_eq!(
        response.disclosure.retention,
        RetentionPolicy::ProviderDefined
    );
    Ok(())
}

#[test]
fn denied_transport_disclosure_posts_zero_bytes() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport::new(
        br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ProviderDisclosure {
            remote: true,
            retention: RetentionPolicy::ProviderDefined,
        },
    )?);
    let identity = fixture_embedding_identity("denied-model", 2)?;
    let mut provider = LocalHttpEmbeddingProvider::with_profile(
        ENDPOINT,
        "denied-model",
        Some(2),
        identity.clone(),
        "{{text}}".to_string(),
        "{{text}}".to_string(),
    )?;
    provider.transport = transport.clone();
    let expected = ProviderDisclosure {
        remote: false,
        retention: RetentionPolicy::NoRetention,
    };
    assert_ne!(provider.disclosure(), expected);
    let denied = provider.disclosure() != expected;
    assert!(denied);
    assert_eq!(transport.post_count.load(Ordering::Relaxed), 0);
    assert!(
        transport
            .body
            .lock()
            .map_err(|_| PortError::Internal {
                message: "recording lock poisoned".to_string(),
            })?
            .is_none()
    );
    Ok(())
}
#[test]
fn rejects_incompatible_request_identity() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Ok(
            br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ))?),
    )?;
    let identity = fixture_embedding_identity("different-model", 2)?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity,
    });
    assert!(result.is_err_and(|error| error.is_invalid_input()));
    Ok(())
}
#[test]
fn rejects_non_loopback_endpoint() -> Result<(), PortError> {
    let result = LocalHttpEmbeddingProvider::with_profile(
        "https://example.com/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        "{{text}}".to_string(),
        "{{text}}".to_string(),
    );
    assert!(result.is_err_and(|error| error.is_invalid_input()));
    Ok(())
}
#[test]
fn parses_and_validates_embedding_response() -> Result<(), PortError> {
    let response = br#"{"data":[{"embedding":[0.1,0.2]}],"model":"model-v1"}"#;
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Ok(response.to_vec()))?),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    })?;
    assert_eq!(result.vector, vec![0.1, 0.2]);
    assert_eq!(result.model_version, "model-v1");
    Ok(())
}
#[test]
fn rejects_empty_text() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Ok(
            br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ))?),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "   ".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        result.as_ref().is_err_and(|error| error.is_invalid_input()),
        "expected InvalidInput for empty text, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_mismatched_model_version() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model-a",
        Some(2),
        fixture_embedding_identity("model-a", 2)?,
        Arc::new(FixtureTransport::new(Ok(
            br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ))?),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model-b".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        result.as_ref().is_err_and(|error| error.is_invalid_input()),
        "expected InvalidInput for mismatched model version, got {result:?}"
    );
    Ok(())
}
#[test]
fn propagates_transport_error() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Err(PortError::Downstream {
            message: "connection refused".to_string(),
        }))?),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
        "expected Downstream error, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_malformed_json_response() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Ok(br#"not-json"#.to_vec()))?),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for malformed JSON, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_empty_embedding_array() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Ok(br#"{"data":[]}"#.to_vec()))?),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for empty data array, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_noncanonical_loopback_paths() {
    for endpoint in [
        "http://localhost:8080/v1/embeddings",
        "http://127.0.0.1:8080/v1/embeddings?token=secret",
        "http://127.0.0.1:8080/v1/embedding",
    ] {
        assert!(parse_loopback_endpoint(endpoint).is_err_and(|error| error.is_invalid_input()));
    }
}

#[test]
fn satisfies_shared_embedding_provider_contract() -> Result<(), Box<dyn std::error::Error>> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        fixture_embedding_identity("model", 2)?,
        Arc::new(FixtureTransport::new(Ok(
            br#"{"data":[{"embedding":[0.1,0.2]}],"model":"model-v1"}"#.to_vec(),
        ))?),
    )?;
    maestria_ports::contract_tests::assert_embedding_provider_contract(&provider)
}
