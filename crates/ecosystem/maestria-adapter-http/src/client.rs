use std::time::Duration;

use maestria_ports::{
    BoundedProviderTransport, PortError, ProviderCallControl, ProviderDisclosure, ProviderEndpoint,
    ProviderTransport, RetentionPolicy,
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
            agent: build_agent(single_timeout),
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
            agent: build_agent(timeout),
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
            .config()
            .timeout_per_call(Some(timeout))
            .build()
            .header("content-type", "application/json")
            .send(body)
            .map_err(|error| PortError::downstream("provider request failed", error.to_string()))?;
        response
            .into_body()
            .read_to_string()
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

impl BoundedProviderTransport for UreqJsonClient {
    fn post_bounded(
        &self,
        body: Vec<u8>,
        control: &dyn ProviderCallControl,
    ) -> Result<Vec<u8>, PortError> {
        const MAX_REQUEST_BYTES: usize = 128 * 1024;
        if body.len() > MAX_REQUEST_BYTES {
            return Err(PortError::invalid_input(
                "bounded provider request",
                "request exceeds configured byte limit",
            ));
        }
        if control.is_cancelled() {
            return Err(PortError::internal(
                "bounded provider request",
                "request was cancelled before send",
            ));
        }
        let remaining_ms = control.remaining_ms();
        if remaining_ms == 0 {
            return Err(PortError::downstream(
                "bounded provider request",
                "request deadline elapsed before send",
            ));
        }
        let response_limit = control.max_response_bytes();
        if response_limit == 0 {
            return Err(PortError::invalid_input(
                "bounded provider response",
                "response limit must be positive",
            ));
        }
        let timeout = self
            .single_timeout
            .min(Duration::from_millis(u64::from(remaining_ms)));
        let endpoint = self.endpoint.as_ref().ok_or_else(|| {
            PortError::internal(
                "bounded provider request",
                "transport has no configured endpoint",
            )
        })?;
        let response = self
            .agent
            .post(endpoint.as_str())
            .config()
            .timeout_per_call(Some(timeout))
            .build()
            .header("content-type", "application/json")
            .send(body)
            .map_err(|error| {
                PortError::downstream("bounded provider request failed", error.to_string())
            })?;
        if control.is_cancelled() {
            return Err(PortError::internal(
                "bounded provider response",
                "request was cancelled after send",
            ));
        }
        let bytes = response
            .into_body()
            .into_with_config()
            .limit((response_limit as u64).saturating_add(1))
            .read_to_vec()
            .map_err(|error| {
                PortError::downstream("read bounded provider response", error.to_string())
            })?;
        if bytes.len() > response_limit {
            return Err(PortError::downstream(
                "bounded provider response",
                "response exceeds configured byte limit",
            ));
        }
        if control.is_cancelled() {
            return Err(PortError::internal(
                "bounded provider response",
                "request was cancelled after read",
            ));
        }
        Ok(bytes)
    }
}

/// Builds the shared agent: no redirects, one global timeout ceiling.
///
/// Per-call deadlines override it via `timeout_per_call` on each request.
fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .build();
    ureq::Agent::new_with_config(config)
}

fn static_fallback_endpoint() -> &'static ProviderEndpoint {
    static ENDPOINT: std::sync::LazyLock<ProviderEndpoint> =
        std::sync::LazyLock::new(ProviderEndpoint::default);
    &ENDPOINT
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_ports::ProviderCallControl;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    struct Control {
        cancelled: bool,
        remaining_ms: u32,
        max_response_bytes: usize,
    }

    impl ProviderCallControl for Control {
        fn is_cancelled(&self) -> bool {
            self.cancelled
        }

        fn remaining_ms(&self) -> u32 {
            self.remaining_ms
        }

        fn max_response_bytes(&self) -> usize {
            self.max_response_bytes
        }
    }

    fn client_for(
        listener: &TcpListener,
        timeout: Duration,
    ) -> Result<UreqJsonClient, Box<dyn std::error::Error>> {
        let address = listener.local_addr()?;
        let endpoint = ProviderEndpoint::loopback_http(
            &format!("http://127.0.0.1:{}/v1/test", address.port()),
            "/v1/test",
        )?;
        Ok(UreqJsonClient::new(endpoint, timeout))
    }

    fn serve_once(
        listener: TcpListener,
        body: &'static [u8],
        delay: Option<Duration>,
    ) -> thread::JoinHandle<Result<(), std::io::Error>> {
        thread::spawn(move || {
            let (mut stream, _) = listener.accept()?;
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            if let Some(delay) = delay {
                thread::sleep(delay);
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
            Ok(())
        })
    }

    #[test]
    fn bounded_transport_rejects_request_body_over_cap() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let client = client_for(&listener, Duration::from_millis(250))?;
        let result = client.post_bounded(
            vec![0; 128 * 1024 + 1],
            &Control {
                cancelled: false,
                remaining_ms: 250,
                max_response_bytes: 16,
            },
        );
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn bounded_transport_rejects_response_over_cap() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let server = serve_once(listener.try_clone()?, b"0123456789", None);
        let client = client_for(&listener, Duration::from_millis(250))?;
        let result = client.post_bounded(
            br#"{}"#.to_vec(),
            &Control {
                cancelled: false,
                remaining_ms: 250,
                max_response_bytes: 8,
            },
        );
        assert!(result.is_err());
        server
            .join()
            .map_err(|_| std::io::Error::other("server thread panicked"))??;
        Ok(())
    }

    #[test]
    fn bounded_transport_applies_remaining_deadline_to_delayed_response()
    -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let server = serve_once(
            listener.try_clone()?,
            br#"{}"#,
            Some(Duration::from_millis(100)),
        );
        let client = client_for(&listener, Duration::from_millis(500))?;
        let result = client.post_bounded(
            br#"{}"#.to_vec(),
            &Control {
                cancelled: false,
                remaining_ms: 10,
                max_response_bytes: 16,
            },
        );
        assert!(result.is_err());
        server
            .join()
            .map_err(|_| std::io::Error::other("server thread panicked"))??;
        Ok(())
    }
}
