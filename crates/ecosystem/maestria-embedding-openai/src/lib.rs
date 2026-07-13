use maestria_ports::{EmbeddingProvider, EmbeddingRequest, EmbeddingResponse, PortError};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use url::Url;
#[derive(Clone)]
pub struct LocalHttpEmbeddingProvider {
    endpoint: Url,
    model: String,
    dimensions: Option<usize>,
    transport: Arc<dyn EmbeddingTransport>,
}

impl LocalHttpEmbeddingProvider {
    pub fn new(endpoint: &str, model: &str, dimensions: Option<usize>) -> Result<Self, PortError> {
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
        Ok(Self {
            endpoint,
            model: model.to_string(),
            dimensions,
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
}

impl EmbeddingProvider for LocalHttpEmbeddingProvider {
    fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse, PortError> {
        if request.text.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "embedding text must not be empty".to_string(),
            });
        }
        let payload = EmbeddingPayload {
            input: request.text,
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
            model_version,
        })
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
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

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
        })?;
        assert_eq!(result.vector, vec![0.1, 0.2]);
        assert_eq!(result.model_version, "model-v1");
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

    #[test]
    fn posts_to_local_http_endpoint() -> Result<(), PortError> {
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|error| PortError::Internal {
                message: format!("bind test embedding server: {error}"),
            })?;
        let port = listener
            .local_addr()
            .map_err(|error| PortError::Internal {
                message: format!("read test embedding server address: {error}"),
            })?
            .port();
        let server = thread::spawn(move || {
            listener.accept().and_then(|(mut stream, _)| {
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request)?;
                let body = br#"{"data":[{"embedding":[0.4,0.6]}],"model":"fake-v1"}"#;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                    body.len()
                )?;
                stream.write_all(body)
            })
        });
        let provider = LocalHttpEmbeddingProvider::new(
            &format!("http://127.0.0.1:{port}/v1/embeddings"),
            "local-model",
            Some(2),
        )?;
        let result = provider.embed(EmbeddingRequest {
            text: "local text".to_string(),
            model: "query".to_string(),
        })?;
        let server_result = server.join().map_err(|_| PortError::Internal {
            message: "test embedding server panicked".to_string(),
        })?;
        server_result.map_err(|error| PortError::Internal {
            message: format!("test embedding server failed: {error}"),
        })?;
        assert_eq!(result.vector, vec![0.4, 0.6]);
        assert_eq!(result.model_version, "fake-v1");
        Ok(())
    }
}
