use super::*;
use std::sync::Mutex;
#[derive(Default)]
struct FixtureTransport {
    response: Mutex<Option<Result<Vec<u8>, PortError>>>,
}
impl FixtureTransport {
    fn new(response: Result<Vec<u8>, PortError>) -> Self {
        Self {
            response: Mutex::new(Some(response)),
        }
    }
}
impl EmbeddingTransport for FixtureTransport {
    fn post(&self, _endpoint: &str, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
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
}
impl EmbeddingTransport for RecordingTransport {
    fn post(&self, _endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        *self.body.lock().map_err(|_| PortError::Internal {
            message: "recording lock poisoned".to_string(),
        })? = Some(body);
        Ok(self.response.clone())
    }
}
#[test]
fn applies_kind_template_and_preserves_disclosure() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport {
        response: br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        body: Mutex::new(None),
    });
    let identity = EmbeddingIdentity::legacy("profiled-model", 2)?;
    let mut provider = LocalHttpEmbeddingProvider::with_profile(
        "http://127.0.0.1:8080/v1/embeddings",
        "profiled-model",
        Some(2),
        identity.clone(),
        "document: {{text}}".to_string(),
        "query: {{text}}".to_string(),
        ProviderDisclosure {
            remote: true,
            retention: RetentionPolicy::ProviderDefined,
        },
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
fn rejects_incompatible_request_identity() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        Arc::new(FixtureTransport::new(Ok(
            br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ))),
    )?;
    let identity = EmbeddingIdentity::legacy("different-model", 2)?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity,
    });
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}
#[test]
fn rejects_non_loopback_endpoint() {
    let result =
        LocalHttpEmbeddingProvider::new("https://example.com/v1/embeddings", "model", None);
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
}
#[test]
fn parses_and_validates_embedding_response() -> Result<(), PortError> {
    let response = br#"{"data":[{"embedding":[0.1,0.2]}],"model":"model-v1"}"#;
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        Arc::new(FixtureTransport::new(Ok(response.to_vec()))),
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
        Arc::new(FixtureTransport::new(Ok(
            br#"{"data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        ))),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "   ".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for empty text, got {result:?}"
    );
    Ok(())
}
#[test]
fn propagates_transport_error() -> Result<(), PortError> {
    let provider = LocalHttpEmbeddingProvider::with_transport(
        "http://127.0.0.1:8080/v1/embeddings",
        "model",
        Some(2),
        Arc::new(FixtureTransport::new(Err(PortError::Downstream {
            message: "connection refused".to_string(),
        }))),
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
        Arc::new(FixtureTransport::new(Ok(br#"not-json"#.to_vec()))),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
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
        Arc::new(FixtureTransport::new(Ok(br#"{"data":[]}"#.to_vec()))),
    )?;
    let result = provider.embed(EmbeddingRequest {
        text: "hello".to_string(),
        model: "model".to_string(),
        kind: EmbeddingInputKind::Document,
        identity: provider.identity().clone(),
    });
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
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
        assert!(matches!(
            parse_loopback_endpoint(endpoint),
            Err(PortError::InvalidInput { .. })
        ));
    }
}
