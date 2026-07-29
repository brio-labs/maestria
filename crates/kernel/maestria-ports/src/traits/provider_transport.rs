use super::{PortError, ProviderDisclosure};

/// A canonical, validated endpoint owned by a provider transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderEndpoint {
    endpoint: String,
}

impl ProviderEndpoint {
    /// Validate a canonical loopback endpoint for one model protocol path.
    pub fn loopback_http(endpoint: &str, path: &'static str) -> Result<Self, PortError> {
        let authority = endpoint
            .strip_prefix("http://")
            .and_then(|endpoint| endpoint.strip_suffix(path));
        let is_loopback = authority.is_some_and(valid_loopback_authority);
        if !path.starts_with('/')
            || path.contains('?')
            || path.contains('#')
            || endpoint.contains('?')
            || endpoint.contains('#')
            || !is_loopback
        {
            return Err(PortError::InvalidInputContext {
                context: "provider endpoint is not canonical loopback",
                source: format!("endpoint must be an http loopback {path} URL"),
            });
        }
        Ok(Self {
            endpoint: endpoint.to_string(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.endpoint
    }
}

fn valid_loopback_authority(authority: &str) -> bool {
    match authority {
        "127.0.0.1" | "[::1]" => true,
        authority => authority
            .strip_prefix("127.0.0.1:")
            .or_else(|| authority.strip_prefix("[::1]:"))
            .is_some_and(valid_port),
    }
}

fn valid_port(port: &str) -> bool {
    !port.is_empty()
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok()
}

/// Shared capability-bearing transport boundary for byte-emitting providers.
///
/// The endpoint and disclosure are stable facts of the object receiving bytes;
/// callers cannot supply either per request or independently of the transport.
pub trait ProviderTransport: Send + Sync {
    fn endpoint(&self) -> &ProviderEndpoint;
    fn disclosure(&self) -> &ProviderDisclosure;
    fn post(&self, body: Vec<u8>) -> Result<Vec<u8>, PortError>;
}
