use crate::rasterizer::{PdfRasterizer, PdftoppmRasterizer};
use crate::transport::{ChatCompletionRequest, ChatCompletionResponse, OcrTransport};
use maestria_ports::{
    OcrIdentity, OcrPage, OcrProvider, OcrRequest, OcrResponse, PortError, ProviderDisclosure,
    RetentionPolicy,
};
use std::sync::Arc;
use url::Url;

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
            Arc::new(maestria_adapter_http::UreqJsonClient::for_timeout(
                std::time::Duration::from_secs(1200),
            )),
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
            return Err(PortError::InvalidInputContext {
                context: "OCR model is empty",
                source: "model must contain a non-whitespace value".to_string(),
            });
        }
        if identity.model != model {
            return Err(PortError::InvalidInputContext {
                context: "OCR model identity mismatch",
                source: "model does not match the provider identity".to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "OCR PDF is empty",
                source: "PDF bytes must contain data".to_string(),
            });
        }
        if request.pages.is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "OCR request pages are empty",
                source: "at least one page is required".to_string(),
            });
        }
        let rendered = self
            .rasterizer
            .rasterize(&request.file.bytes, &request.pages)?;
        let mut pages = Vec::with_capacity(rendered.len());
        for page in rendered {
            if page.bytes.is_empty() {
                return Err(PortError::InvalidInputContext {
                    context: "rasterized OCR page is empty",
                    source: format!("page {} contains no image bytes", page.page),
                });
            }
            let payload = ChatCompletionRequest::for_image(
                &self.model,
                DEFAULT_PROMPT,
                &page.mime_type,
                &page.bytes,
            );
            let body = serde_json::to_vec(&payload)
                .map_err(|error| PortError::internal("encode OCR request", error.to_string()))?;
            let response = self.transport.post(self.endpoint.as_str(), body)?;
            let parsed: ChatCompletionResponse =
                serde_json::from_slice(&response).map_err(|error| {
                    PortError::DownstreamContext {
                        context: "decode OCR response",
                        source: error.to_string(),
                    }
                })?;
            let text = parsed.text().ok_or_else(|| {
                PortError::downstream(
                    "decode OCR response text",
                    format!("OCR response contained no text for page {}", page.page),
                )
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

    fn identity(&self) -> OcrIdentity {
        self.identity.clone()
    }

    fn disclosure(&self) -> ProviderDisclosure {
        self.disclosure.clone()
    }
}

fn parse_loopback_endpoint(endpoint: &str) -> Result<Url, PortError> {
    let url = Url::parse(endpoint)
        .map_err(|error| PortError::invalid_input("invalid OCR endpoint", error.to_string()))?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/chat/completions"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(PortError::InvalidInputContext {
            context: "OCR endpoint is not canonical loopback",
            source: "endpoint must be an http loopback /v1/chat/completions URL".to_string(),
        });
    }
    Ok(url)
}

#[cfg(test)]
mod tests;
