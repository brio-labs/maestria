#![forbid(unsafe_code)]

//! Synchronous web evidence adapter backed by `ureq`.
//!
//! Note: This adapter explicitly implements only the basic transport contract (`WebFetcher`).
//! It does not perform HTML title extraction, provenance recording, or prompt-injection handling.
//! These policies, along with runtime snapshot persistence and trust boundaries, are deferred
//! to the orchestrating runtime (e.g., `maestria-harness` and domain governance) to keep
//! the network boundary decoupled from execution policies.

use maestria_ports::{PortError, WebFetcher, WebSnapshotData};
use std::time::Duration;

trait HttpTransport: Send + Sync + std::fmt::Debug {
    fn get(&self, url: &str) -> Result<String, PortError>;
}

#[derive(Clone, Debug)]
struct UreqTransport {
    agent: ureq::Agent,
}

impl HttpTransport for UreqTransport {
    fn get(&self, url: &str) -> Result<String, PortError> {
        let response = match self.agent.get(url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(404, _)) => return Err(PortError::NotFound),
            Err(e) => return Err(downstream_error(e)),
        };
        response.into_string().map_err(downstream_error)
    }
}

#[derive(Clone, Debug)]
pub struct UreqWebFetcher {
    transport: std::sync::Arc<dyn HttpTransport>,
}

impl Default for UreqWebFetcher {
    fn default() -> Self {
        Self {
            transport: std::sync::Arc::new(UreqTransport {
                agent: ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(15))
                    .build(),
            }),
        }
    }
}

impl UreqWebFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_transport(transport: std::sync::Arc<dyn HttpTransport>) -> Self {
        Self { transport }
    }
}

impl WebFetcher for UreqWebFetcher {
    fn fetch(&self, url_str: &str) -> Result<WebSnapshotData, PortError> {
        let parsed = url::Url::parse(url_str).map_err(|e| PortError::InvalidInput {
            message: format!("invalid url: {}", e),
        })?;

        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(PortError::InvalidInput {
                message: "url must have http or https scheme".to_string(),
            });
        }

        if parsed.host_str().is_none_or(str::is_empty) {
            return Err(PortError::InvalidInput {
                message: "url must have a host".to_string(),
            });
        }

        let html = self.transport.get(url_str)?;

        Ok(WebSnapshotData {
            url: url_str.to_string(),
            html,
        })
    }
}

fn downstream_error(error: impl std::fmt::Display) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Debug)]
    struct FixtureTransport {
        responses: BTreeMap<String, Result<String, PortError>>,
    }

    impl FixtureTransport {
        fn new(responses: BTreeMap<String, Result<String, PortError>>) -> Self {
            Self { responses }
        }
    }

    impl HttpTransport for FixtureTransport {
        fn get(&self, url: &str) -> Result<String, PortError> {
            if let Some(res) = self.responses.get(url) {
                match res {
                    Ok(s) => Ok(s.clone()),
                    Err(PortError::NotFound) => Err(PortError::NotFound),
                    Err(PortError::Downstream { message }) => Err(PortError::Downstream {
                        message: message.clone(),
                    }),
                    _ => Err(downstream_error("unknown fixture error")),
                }
            } else {
                Err(PortError::Downstream {
                    message: "connection refused".to_string(),
                })
            }
        }
    }

    #[test]
    fn test_fetch_success() -> Result<(), Box<dyn std::error::Error>> {
        let url = "http://example.com/test";
        let mut responses = BTreeMap::new();
        responses.insert(
            url.to_string(),
            Ok("<html><body>hello</body></html>".to_string()),
        );
        let fetcher =
            UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)));

        let data = fetcher.fetch(url)?;
        assert_eq!(data.url, url);
        assert_eq!(data.html, "<html><body>hello</body></html>");
        Ok(())
    }

    #[test]
    fn test_fetch_not_found() -> Result<(), Box<dyn std::error::Error>> {
        let url = "http://example.com/test-404";
        let mut responses = BTreeMap::new();
        responses.insert(url.to_string(), Err(PortError::NotFound));
        let fetcher =
            UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)));

        let err = fetcher.fetch(url);
        assert!(matches!(err, Err(PortError::NotFound)));
        Ok(())
    }

    #[test]
    fn test_fetch_invalid_url() -> Result<(), Box<dyn std::error::Error>> {
        let fetcher = UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(
            BTreeMap::new(),
        )));

        let invalid_urls = vec![
            "",
            "ftp://example.com",
            "http://",
            "https://",
            "http:// ",
            "http://\n",
            "not-a-url",
        ];

        for url in invalid_urls {
            assert!(
                matches!(fetcher.fetch(url), Err(PortError::InvalidInput { .. })),
                "Expected InvalidInput for url: '{}'",
                url
            );
        }
        Ok(())
    }

    #[test]
    fn test_fetch_connection_refused() -> Result<(), Box<dyn std::error::Error>> {
        let fetcher = UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(
            BTreeMap::new(),
        )));
        let err = fetcher.fetch("http://127.0.0.1:12345");
        assert!(matches!(err, Err(PortError::Downstream { .. })));
        Ok(())
    }

    #[test]
    fn test_contract() -> Result<(), Box<dyn std::error::Error>> {
        let url = "http://example.com/contract";
        let html = "<html><body>hello</body></html>";
        let mut responses = BTreeMap::new();
        responses.insert(url.to_string(), Ok(html.to_string()));
        let fetcher =
            UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)));

        maestria_ports::contract_tests::assert_web_fetcher_contract(&fetcher, url, html)?;
        Ok(())
    }
}
