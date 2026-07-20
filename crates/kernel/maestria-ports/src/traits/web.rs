#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebSnapshotData {
    pub url: String,
    pub html: String,
    pub content_hash: String,
    pub metadata: maestria_domain::WebEvidenceMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFetchOptions {
    pub max_bytes: usize,
    pub max_latency_ms: u32,
    pub allowed_domains: Vec<String>,
    pub allowed_content_types: Vec<String>,
}

impl WebFetchOptions {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            max_latency_ms: 15_000,
            allowed_domains: Vec::new(),
            allowed_content_types: Vec::new(),
        }
    }
}

pub trait WebFetcher: Send + Sync {
    /// Fetches at most `options.max_bytes` of response body data.
    fn fetch(&self, url: &str, max_bytes: usize) -> Result<WebSnapshotData, crate::PortError>;

    fn fetch_with_options(
        &self,
        url: &str,
        options: &WebFetchOptions,
    ) -> Result<WebSnapshotData, crate::PortError> {
        self.fetch(url, options.max_bytes)
    }
}
