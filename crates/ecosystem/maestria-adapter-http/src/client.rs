use std::time::Duration;

use maestria_ports::{
    PortError, ProviderDisclosure, ProviderEndpoint, ProviderTransport, RetentionPolicy,
};

/// Shared `ureq`-backed HTTP transport for model provider adapters.
#[derive(Debug, Clone)]
pub struct UreqJsonClient {
    endpoint: Option<ProviderEndpoint>,
    disclosure: ProviderDisclosure,
    agent: ureq::Agent,
    single_timeout: Duration,
    batch_timeout: Duration,
}

impl UreqJsonClient {
    /// Creates a new JSON transport with the given request deadline.
    pub fn new(endpoint: ProviderEndpoint, timeout: Duration) -> Self {
        Self::with_batch_timeout(endpoint, timeout, timeout)
    }

    /// Creates a new JSON transport with distinct single and batch request deadlines.
    pub fn with_batch_timeout(
        endpoint: ProviderEndpoint,
        single_timeout: Duration,
        batch_timeout: Duration,
    ) -> Self {
        Self {
            endpoint: Some(endpoint),
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
            agent: ureq::AgentBuilder::new()
                .timeout(single_timeout)
                .redirects(0)
                .build(),
            single_timeout,
            batch_timeout,
        }
    }

    /// Creates a transport configured with a timeout for dynamic URL posting.
    pub fn for_timeout(timeout: Duration) -> Self {
        Self {
            endpoint: None,
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
            agent: ureq::AgentBuilder::new()
                .timeout(timeout)
                .redirects(0)
                .build(),
            single_timeout: timeout,
            batch_timeout: timeout,
        }
    }

    /// Posts raw bytes to a specific URL with the configured single-request timeout.
    pub fn post_url(&self, url: &str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        self.post_bytes(url, body, self.single_timeout)
    }

    fn post_bytes(
        &self,
        url: &str,
        body: Vec<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>, PortError> {
        let response = self
            .agent
            .post(url)
            .timeout(timeout)
            .set("content-type", "application/json")
            .send_bytes(&body)
            .map_err(|error| PortError::downstream("provider request failed", error.to_string()))?;
        response
            .into_string()
            .map(String::into_bytes)
            .map_err(|error| PortError::downstream("read provider response", error.to_string()))
    }
}

impl ProviderTransport for UreqJsonClient {
    fn endpoint(&self) -> &ProviderEndpoint {
        match &self.endpoint {
            Some(endpoint) => endpoint,
            None => static_fallback_endpoint(),
        }
    }

    fn disclosure(&self) -> &ProviderDisclosure {
        &self.disclosure
    }

    fn post(&self, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        let endpoint = match &self.endpoint {
            Some(ep) => ep.as_str(),
            None => {
                return Err(PortError::internal(
                    "provider transport post",
                    "transport has no configured endpoint",
                ));
            }
        };
        self.post_bytes(endpoint, body, self.single_timeout)
    }

    fn post_to(&self, path_suffix: &'static str, body: Vec<u8>) -> Result<Vec<u8>, PortError> {
        let endpoint = match &self.endpoint {
            Some(ep) => ep.as_str(),
            None => {
                return Err(PortError::internal(
                    "provider transport post_to",
                    "transport has no configured endpoint",
                ));
            }
        };
        let url = format!("{}{}", endpoint, path_suffix);
        self.post_bytes(&url, body, self.batch_timeout)
    }
}

fn static_fallback_endpoint() -> &'static ProviderEndpoint {
    static ENDPOINT: std::sync::LazyLock<ProviderEndpoint> =
        std::sync::LazyLock::new(ProviderEndpoint::default);
    &ENDPOINT
}
