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
        if request.file.bytes.is_empty() {
            return Err(PortError::InvalidInput {
                message: "cannot OCR an empty PDF".to_string(),
            });
        }
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
            if page.bytes.is_empty() {
                return Err(PortError::InvalidInput {
                    message: format!("rasterized page {} is empty", page.page),
                });
            }
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
mod tests;
