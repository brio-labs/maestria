use super::*;
use maestria_ports::learned_sparse_contract_tests::fixture_sparse_identity;
use std::net::TcpListener;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

const ENDPOINT: &str = "http://127.0.0.1:10002/v1/sparse";

fn endpoint() -> Result<ProviderEndpoint, PortError> {
    ProviderEndpoint::loopback_http(ENDPOINT, SPARSE_ENDPOINT_PATH)
}

const VALID_RESPONSE: &[u8] = br#"{"model":"splade","term_ids":[1,2],"weights":[0.5,0.25]}"#;

#[test]
fn local_http_sparse_provider_passes_shared_contract() -> Result<(), Box<dyn std::error::Error>> {
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        Arc::new(RecordingTransport::new(VALID_RESPONSE.to_vec())?),
    )?;
    maestria_ports::learned_sparse_contract_tests::assert_learned_sparse_provider_contract(
        &provider,
    )
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

#[test]
fn rejects_non_loopback_endpoint() -> Result<(), PortError> {
    let result = LocalHttpSparseProvider::new(
        "https://example.com/v1/sparse",
        "fixture-sparse",
        fixture_sparse_identity()?,
    );
    assert!(result.is_err_and(|error| error.is_invalid_input()));
    Ok(())
}

#[test]
fn rejects_empty_query() -> Result<(), PortError> {
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        Arc::new(RecordingTransport::new(VALID_RESPONSE.to_vec())?),
    )?;
    let result = provider.encode("   ", SparseInputKind::Query, fixture_sparse_identity()?);
    assert!(
        result.as_ref().is_err_and(|error| error.is_invalid_input()),
        "expected InvalidInput for empty text, got {result:?}"
    );
    Ok(())
}

#[test]
fn server_down_returns_typed_failure() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let endpoint = format!("http://127.0.0.1:{port}/v1/sparse");
    let provider =
        LocalHttpSparseProvider::new(&endpoint, "fixture-sparse", fixture_sparse_identity()?)?;
    let result = provider.encode(
        "table latency",
        SparseInputKind::Query,
        fixture_sparse_identity()?,
    );
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for unreachable server, got {result:?}"
    );
    Ok(())
}

#[test]
fn propagates_transport_error() -> Result<(), PortError> {
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        Arc::new(StaticTransport::new(Err(PortError::Downstream {
            message: "sparse transport failed".to_string(),
        }))?),
    )?;
    let result = provider.encode(
        "table latency",
        SparseInputKind::Query,
        fixture_sparse_identity()?,
    );
    assert!(
        matches!(result, Err(PortError::Downstream { .. })),
        "expected Downstream error, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_malformed_json_response() -> Result<(), PortError> {
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        Arc::new(StaticTransport::new(Ok(br#"not-json"#.to_vec()))?),
    )?;
    let result = provider.encode(
        "table latency",
        SparseInputKind::Query,
        fixture_sparse_identity()?,
    );
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for malformed JSON, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_duplicate_terms_in_response() -> Result<(), PortError> {
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        Arc::new(StaticTransport::new(Ok(
            br#"{"model":"splade","term_ids":[1,1],"weights":[0.5,0.25]}"#.to_vec(),
        ))?),
    )?;
    let result = provider.encode(
        "table latency",
        SparseInputKind::Query,
        fixture_sparse_identity()?,
    );
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for duplicate terms, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_oversized_vector_response() -> Result<(), PortError> {
    // The fixture fingerprint allows 128 terms; craft an oversized-but-valid
    // response by lowering the term cap through a custom identity instead.
    let mut oversized = fixture_sparse_identity()?;
    oversized.fingerprint.max_terms = 2;
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        oversized.clone(),
        Arc::new(RecordingTransport::new(
            br#"{"model":"splade","term_ids":[1,2,3],"weights":[0.5,0.25,0.125]}"#.to_vec(),
        )?),
    )?;
    let result = provider.encode("table latency", SparseInputKind::Query, oversized);
    assert!(
        matches!(result, Err(PortError::DownstreamContext { .. })),
        "expected Downstream error for oversized vector, got {result:?}"
    );
    Ok(())
}

#[test]
fn denied_transport_disclosure_posts_zero_bytes() -> Result<(), PortError> {
    let transport = Arc::new(StaticTransport::denied(Ok(VALID_RESPONSE.to_vec()))?);
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        transport.clone(),
    )?;
    let expected = ProviderDisclosure {
        remote: false,
        retention: RetentionPolicy::NoRetention,
    };
    assert_ne!(provider.disclosure().as_ref(), Some(&expected));
    assert_eq!(transport.post_count.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn disclosure_matches_local_no_retention() -> Result<(), PortError> {
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        Arc::new(RecordingTransport::new(VALID_RESPONSE.to_vec())?),
    )?;
    assert_eq!(
        provider.disclosure(),
        Some(ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        })
    );
    Ok(())
}

#[test]
fn sends_text_and_kind_and_preserves_identity() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport::new(VALID_RESPONSE.to_vec())?);
    let expected_identity = fixture_sparse_identity()?;
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        expected_identity.clone(),
        transport.clone(),
    )?;
    let vector = provider.encode(
        "table latency",
        SparseInputKind::Query,
        expected_identity.clone(),
    )?;
    assert_eq!(vector.identity(), &expected_identity);
    assert_eq!(vector.terms().len(), 2);
    let body = transport
        .body
        .lock()
        .map_err(|_| PortError::Internal {
            message: "recording mutex poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| PortError::Internal {
            message: "request body was not recorded".to_string(),
        })?;
    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| PortError::Internal {
            message: format!("decode recorded request: {error}"),
        })?;
    assert_eq!(payload["text"], "table latency");
    assert_eq!(payload["kind"], "query");
    Ok(())
}

#[test]
fn document_kind_is_serialized_on_the_wire() -> Result<(), PortError> {
    let transport = Arc::new(RecordingTransport::new(VALID_RESPONSE.to_vec())?);
    let provider = LocalHttpSparseProvider::with_transport(
        ENDPOINT,
        "fixture-sparse",
        fixture_sparse_identity()?,
        transport.clone(),
    )?;
    provider.encode(
        "fn main() {}",
        SparseInputKind::Document,
        fixture_sparse_identity()?,
    )?;
    let body = transport
        .body
        .lock()
        .map_err(|_| PortError::Internal {
            message: "recording mutex poisoned".to_string(),
        })?
        .clone()
        .ok_or_else(|| PortError::Internal {
            message: "request body was not recorded".to_string(),
        })?;
    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|error| PortError::Internal {
            message: format!("decode recorded request: {error}"),
        })?;
    assert_eq!(payload["text"], "fn main() {}");
    assert_eq!(payload["kind"], "document");
    Ok(())
}
