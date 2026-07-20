#![forbid(unsafe_code)]

mod rasterizer;
mod transport;

pub use rasterizer::{PdfRasterizer, PdftoppmRasterizer, RasterizedPage};
pub use transport::{OcrTransport, UreqTransport};

use maestria_ports::{
    OcrIdentity, OcrPage, OcrProvider, OcrRequest, OcrResponse, PortError, ProviderDisclosure,
    RetentionPolicy,
};
use std::sync::Arc;
use url::Url;

use transport::{ChatCompletionRequest, ChatCompletionResponse};

const DEFAULT_PROMPT: &str = "document parsing.";

#[derive(Clone)]
pub struct LocalHttpOcrProvider {
    endpoint: Url,
    model: String,
    identity: OcrIdentity,
    disclosure: ProviderDisclosure,
    rasterizer: Arc<dyn PdfRasterizer>,
    transport: Arc<dyn OcrTransport>,
}

impl LocalHttpOcrProvider {
    pub fn new(endpoint: &str, model: &str, identity: OcrIdentity) -> Result<Self, PortError> {
        Self::with_parts(
            endpoint,
            model,
            identity,
            Arc::new(PdftoppmRasterizer),
            Arc::new(UreqTransport::default()),
        )
    }

    pub fn with_parts(
        endpoint: &str,
        model: &str,
        identity: OcrIdentity,
        rasterizer: Arc<dyn PdfRasterizer>,
        transport: Arc<dyn OcrTransport>,
    ) -> Result<Self, PortError> {
        let endpoint = parse_loopback_endpoint(endpoint)?;
        if model.trim().is_empty() {
            return Err(PortError::InvalidInput {
                message: "OCR model must not be empty".to_string(),
            });
        }
        if identity.model != model {
            return Err(PortError::InvalidInput {
                message: "OCR model does not match provider identity".to_string(),
            });
        }
        Ok(Self {
            endpoint,
            model: model.to_string(),
            identity,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
            rasterizer,
            transport,
        })
    }

    pub fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn check_local_tools(&self) -> Result<(), PortError> {
        self.rasterizer.check_available()
    }
}

impl OcrProvider for LocalHttpOcrProvider {
    fn recognize(&self, request: OcrRequest) -> Result<OcrResponse, PortError> {
        if request.pages.is_empty() {
            return Err(PortError::InvalidInput {
                message: "OCR request must contain at least one page".to_string(),
            });
        }
        let rendered = self
            .rasterizer
            .rasterize(&request.file.bytes, &request.pages)?;
        let mut pages = Vec::with_capacity(rendered.len());
        for page in rendered {
            let payload = ChatCompletionRequest::for_image(
                &self.model,
                DEFAULT_PROMPT,
                &page.mime_type,
                &page.bytes,
            );
            let body = serde_json::to_vec(&payload).map_err(|error| PortError::Internal {
                message: format!("encode OCR request: {error}"),
            })?;
            let response = self.transport.post(self.endpoint.as_str(), body)?;
            let parsed: ChatCompletionResponse =
                serde_json::from_slice(&response).map_err(|error| PortError::Downstream {
                    message: format!("decode OCR response for page {}: {error}", page.page),
                })?;
            let text = parsed.text().ok_or_else(|| PortError::Downstream {
                message: format!("OCR response contained no text for page {}", page.page),
            })?;
            pages.push(OcrPage {
                page: page.page,
                text,
            });
        }
        Ok(OcrResponse {
            pages,
            identity: self.identity.clone(),
            disclosure: self.disclosure.clone(),
        })
    }

    fn identity(&self) -> Option<OcrIdentity> {
        Some(self.identity.clone())
    }

    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(self.disclosure.clone())
    }
}

fn parse_loopback_endpoint(endpoint: &str) -> Result<Url, PortError> {
    let url = Url::parse(endpoint).map_err(|error| PortError::InvalidInput {
        message: format!("invalid OCR endpoint: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/chat/completions"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(PortError::InvalidInput {
            message: "OCR endpoint must be an http loopback /v1/chat/completions URL".to_string(),
        });
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_ports::FileHandle;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FixtureRasterizer;

    impl PdfRasterizer for FixtureRasterizer {
        fn rasterize(&self, _pdf: &[u8], pages: &[u32]) -> Result<Vec<RasterizedPage>, PortError> {
            Ok(pages
                .iter()
                .map(|page| RasterizedPage {
                    page: *page,
                    mime_type: "image/png".to_string(),
                    bytes: format!("page-{page}").into_bytes(),
                })
                .collect())
        }

        fn check_available(&self) -> Result<(), PortError> {
            Ok(())
        }
    }

    struct FixtureTransport {
        requests: Mutex<Vec<Vec<u8>>>,
    }

    impl OcrTransport for FixtureTransport {
        fn post(&self, _endpoint: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
            self.requests
                .lock()
                .map_err(|_| PortError::Internal {
                    message: "fixture transport mutex poisoned".to_string(),
                })?
                .push(body);
            Ok(br#"{"choices":[{"message":{"content":"recognized page"}}]}"#.to_vec())
        }
    }

    fn identity() -> OcrIdentity {
        OcrIdentity {
            provider: "baidu".to_string(),
            model: "Unlimited-OCR".to_string(),
            revision: "main".to_string(),
            artifact_hash:
                "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            preprocessing_version: "pdf-pdftoppm-v1".to_string(),
        }
    }

    #[test]
    fn rejects_non_loopback_endpoints() {
        let result = LocalHttpOcrProvider::with_parts(
            "https://example.com/v1/chat/completions",
            "Unlimited-OCR",
            identity(),
            Arc::new(FixtureRasterizer),
            Arc::new(FixtureTransport {
                requests: Mutex::new(Vec::new()),
            }),
        );
        assert!(matches!(result, Err(PortError::InvalidInput { .. })));
    }

    #[test]
    fn sends_one_image_request_per_page_and_preserves_identity() -> Result<(), PortError> {
        let transport = Arc::new(FixtureTransport {
            requests: Mutex::new(Vec::new()),
        });
        let provider = LocalHttpOcrProvider::with_parts(
            "http://127.0.0.1:10000/v1/chat/completions",
            "Unlimited-OCR",
            identity(),
            Arc::new(FixtureRasterizer),
            transport.clone(),
        )?;
        let response = provider.recognize(OcrRequest {
            file: FileHandle {
                path: PathBuf::from("scan.pdf"),
                bytes: b"pdf".to_vec(),
            },
            pages: vec![1, 3],
        })?;
        assert_eq!(response.pages.len(), 2);
        assert_eq!(response.pages[1].page, 3);
        assert_eq!(response.pages[0].text, "recognized page");
        assert_eq!(response.identity, identity());
        assert_eq!(
            transport
                .requests
                .lock()
                .map_err(|_| PortError::Internal {
                    message: "fixture transport mutex poisoned".to_string(),
                })?
                .len(),
            2
        );
        Ok(())
    }
}
