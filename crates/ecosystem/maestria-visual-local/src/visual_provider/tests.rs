use super::*;
use maestria_domain::{BlobId, ContentHash, IndexFingerprint, IndexGenerationId};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const ENDPOINT: &str = "http://127.0.0.1:10001/v1/embeddings";

fn endpoint() -> Result<ProviderEndpoint, PortError> {
    ProviderEndpoint::loopback_http(ENDPOINT, VISUAL_ENDPOINT_PATH)
}

fn disclosure() -> ProviderDisclosure {
    ProviderDisclosure {
        remote: false,
        retention: RetentionPolicy::NoRetention,
    }
}

struct RecordingTransport {
    body: Mutex<Option<Vec<u8>>>,
    response: Vec<u8>,
    endpoint: ProviderEndpoint,
    disclosure: ProviderDisclosure,
}

impl RecordingTransport {
    fn new(response: Vec<u8>) -> Result<Self, PortError> {
        Ok(Self {
            body: Mutex::new(None),
            response,
            endpoint: endpoint()?,
            disclosure: disclosure(),
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
        *self.body.lock().map_err(|_| PortError::Internal {
            message: "recording mutex poisoned".to_string(),
        })? = Some(body);
        Ok(self.response.clone())
    }
}

struct StaticTransport {
    response: Mutex<Option<Result<Vec<u8>, PortError>>>,
    post_count: AtomicUsize,
    endpoint: ProviderEndpoint,
    disclosure: ProviderDisclosure,
}

impl StaticTransport {
    fn new(response: Result<Vec<u8>, PortError>) -> Result<Self, PortError> {
        Ok(Self {
            response: Mutex::new(Some(response)),
            post_count: AtomicUsize::new(0),
            endpoint: endpoint()?,
            disclosure: disclosure(),
        })
    }

    fn denied(response: Result<Vec<u8>, PortError>) -> Result<Self, PortError> {
        Ok(Self {
            response: Mutex::new(Some(response)),
            post_count: AtomicUsize::new(0),
            endpoint: endpoint()?,
            disclosure: ProviderDisclosure {
                remote: true,
                retention: RetentionPolicy::ProviderDefined,
            },
        })
    }
}

impl ProviderTransport for StaticTransport {
    fn endpoint(&self) -> &ProviderEndpoint {
        &self.endpoint
    }

    fn disclosure(&self) -> &ProviderDisclosure {
        &self.disclosure
    }

    fn post(&self, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.post_count.fetch_add(1, Ordering::Relaxed);
        self.response
            .lock()
            .map_err(|_| PortError::Internal {
                message: "static mutex poisoned".to_string(),
            })?
            .take()
            .ok_or_else(|| PortError::Internal {
                message: "static response already consumed".to_string(),
            })?
    }
}
fn identity() -> Result<EmbeddingIdentity, PortError> {
    let artifact_hash =
        ContentHash::new(format!("sha256:{}", "0".repeat(64))).map_err(|error| {
            PortError::Internal {
                message: format!("create test artifact hash: {error}"),
            }
        })?;
    Ok(EmbeddingIdentity {
        generation_id: IndexGenerationId::new(1),
        fingerprint: IndexFingerprint {
            provider: "siglip-onnx".to_string(),
            model: "siglip-v1".to_string(),
            revision: "r1".to_string(),
            artifact_hash,
            dimensions: 2,
            quantization: "int8".to_string(),
            query_template_hash: "query-r1".to_string(),
            document_template_hash: "document-r1".to_string(),
            preprocessing_version: "siglip-224-r1".to_string(),
        },
        representation: RepresentationName::new("visual_page_v1"),
    })
}
#[test]
fn rejects_non_loopback_endpoint() -> Result<(), PortError> {
    let result = LocalHttpVisualProvider::new(
        "https://example.com/v1/embeddings",
        "siglip-v1",
        identity()?,
    );
    assert!(result.is_err_and(|error| error.is_invalid_input()));
    Ok(())
}
#[test]
fn rejects_empty_query() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        "http://127.0.0.1:10001/v1/embeddings",
        "siglip-v1",
        identity()?,
        Arc::new(RecordingTransport::new(
            br#"{"model":"siglip-v1","data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        )?),
    )?;
    let result = provider.embed_query("   ", identity()?);
    assert!(
        result.as_ref().is_err_and(|error| error.is_invalid_input()),
        "expected InvalidInput for empty query, got {result:?}"
    );
    Ok(())
}
#[test]
fn denied_transport_disclosure_posts_zero_bytes() -> Result<(), PortError> {
    let transport = Arc::new(StaticTransport::denied(Ok(
        br#"{"model":"siglip-v1","data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
    ))?);
    let provider = LocalHttpVisualProvider::with_transport(
        ENDPOINT,
        "siglip-v1",
        identity()?,
        transport.clone(),
    )?;
    let expected = ProviderDisclosure {
        remote: false,
        retention: RetentionPolicy::NoRetention,
    };
    assert_ne!(provider.disclosure(), expected);
    assert!(provider.disclosure() != expected);
    assert_eq!(transport.post_count.load(Ordering::Relaxed), 0);
    Ok(())
}
#[test]
fn propagates_transport_error() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        ENDPOINT,
        "siglip-v1",
        identity()?,
        Arc::new(StaticTransport::new(Err(PortError::Downstream {
            message: "visual transport failed".to_string(),
        }))?),
    )?;
    let result = provider.embed_query("table latency", identity()?);
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
        "expected Downstream error, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_malformed_json_response() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        ENDPOINT,
        "siglip-v1",
        identity()?,
        Arc::new(StaticTransport::new(Ok(br#"not-json"#.to_vec()))?),
    )?;
    let result = provider.embed_query("table latency", identity()?);
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for malformed JSON, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_empty_source_bytes() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        ENDPOINT,
        "siglip-v1",
        identity()?,
        Arc::new(RecordingTransport::new(
            br#"{"model":"siglip-v1","data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
        )?),
    )?;
    let result = provider.embed_source(maestria_ports::VisualEmbeddingRequest {
        source: maestria_ports::VisualSource::Page {
            blob: BlobId::new(1),
            page_start: 1,
            page_end: 1,
        },
        bytes: vec![],
        identity: identity()?,
    });
    assert!(
        result.as_ref().is_err_and(|error| error.is_invalid_input()),
        "expected InvalidInput for empty source bytes, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_missing_embedding_in_response() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        ENDPOINT,
        "siglip-v1",
        identity()?,
        Arc::new(StaticTransport::new(Ok(
            br#"{"model":"siglip-v1","data":[]}"#.to_vec(),
        ))?),
    )?;
    let result = provider.embed_query("table latency", identity()?);
    assert!(
        matches!(result.as_ref(), Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for missing embedding, got {result:?}"
    );
    Ok(())
}

#[test]
fn sends_text_query_and_preserves_identity() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport::new(
        br#"{"model":"siglip-v1","data":[{"embedding":[0.1,0.2]}]}"#.to_vec(),
    )?);
    let expected_identity = identity()?;
    let provider = LocalHttpVisualProvider::with_transport(
        ENDPOINT,
        "siglip-v1",
        expected_identity.clone(),
        transport.clone(),
    )?;
    let response = provider.embed_query("table latency", expected_identity)?;
    assert_eq!(response.vector, vec![0.1, 0.2]);
    assert_eq!(response.model_version, "siglip-v1");
    let body = transport
        .body
        .lock()
        .map_err(|_| PortError::Internal {
            message: "recording mutex poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| PortError::Internal {
            message: "missing request body".to_string(),
        })?;
    let body = String::from_utf8(body).map_err(|error| PortError::Internal {
        message: format!("request body was not UTF-8: {error}"),
    })?;
    assert!(body.contains("table latency"));
    Ok(())
}
