use super::*;
use maestria_domain::{BlobId, ContentHash, IndexFingerprint, IndexGenerationId};
use std::sync::Mutex;
#[derive(Default)]
struct RecordingTransport {
    body: Mutex<Option<Vec<u8>>>,
}
impl VisualTransport for RecordingTransport {
    fn post(&self, _endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        *self.body.lock().map_err(|_| PortError::Internal {
            message: "recording mutex poisoned".to_string(),
        })? = Some(body);
        Ok(br#"{"model":"siglip-v1","data":[{"embedding":[0.1,0.2]}]}"#.to_vec())
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
    assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    Ok(())
}
#[test]
fn rejects_empty_query() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        "http://127.0.0.1:10001/v1/embeddings",
        "siglip-v1",
        identity()?,
        Arc::new(RecordingTransport::default()),
    )?;
    let result = provider.embed_query("   ", identity()?);
    assert!(
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for empty query, got {result:?}"
    );
    Ok(())
}
#[test]
fn propagates_transport_error() -> Result<(), PortError> {
    struct ErrorTransport;
    impl VisualTransport for ErrorTransport {
        fn post(&self, _endpoint: &str, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
            Err(PortError::Downstream {
                message: "visual transport failed".to_string(),
            })
        }
    }
    let provider = LocalHttpVisualProvider::with_transport(
        "http://127.0.0.1:10001/v1/embeddings",
        "siglip-v1",
        identity()?,
        Arc::new(ErrorTransport),
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
    struct MalformedTransport;
    impl VisualTransport for MalformedTransport {
        fn post(&self, _endpoint: &str, _body: Vec<u8>) -> Result<Vec<u8>, PortError> {
            Ok(br#"not-json"#.to_vec())
        }
    }
    let provider = LocalHttpVisualProvider::with_transport(
        "http://127.0.0.1:10001/v1/embeddings",
        "siglip-v1",
        identity()?,
        Arc::new(MalformedTransport),
    )?;
    let result = provider.embed_query("table latency", identity()?);
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
        "expected Downstream error for malformed JSON, got {result:?}"
    );
    Ok(())
}
#[test]
fn rejects_empty_source_bytes() -> Result<(), PortError> {
    let provider = LocalHttpVisualProvider::with_transport(
        "http://127.0.0.1:10001/v1/embeddings",
        "siglip-v1",
        identity()?,
        Arc::new(RecordingTransport::default()),
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
        matches!(result, Err(PortError::InvalidInput { .. })),
        "expected InvalidInput for empty source bytes, got {result:?}"
    );
    Ok(())
}
#[test]
fn sends_text_query_and_preserves_identity() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport::default());
    let expected_identity = identity()?;
    let provider = LocalHttpVisualProvider::with_transport(
        "http://127.0.0.1:10001/v1/embeddings",
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
