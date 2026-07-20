use super::{FileHandle, PortError, ProviderDisclosure};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrIdentity {
    pub provider: String,
    pub model: String,
    pub revision: String,
    pub artifact_hash: String,
    pub preprocessing_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrRequest {
    pub file: FileHandle,
    pub pages: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrPage {
    pub page: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrResponse {
    pub pages: Vec<OcrPage>,
    pub identity: OcrIdentity,
    pub disclosure: ProviderDisclosure,
}

/// Optional OCR boundary for scanned documents.
///
/// Providers return page-scoped text only. They must not invent PDF region
/// coordinates; pages without a provider remain an explicit `NeedsOcr`
/// degradation in the parser result.
pub trait OcrProvider: Send + Sync {
    fn recognize(&self, request: OcrRequest) -> Result<OcrResponse, PortError>;
    fn identity(&self) -> Option<OcrIdentity>;
    fn disclosure(&self) -> Option<ProviderDisclosure>;
}
