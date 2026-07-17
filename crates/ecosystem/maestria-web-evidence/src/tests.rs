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
    fn get(&self, url: &str, max_bytes: usize) -> Result<HttpResponse, PortError> {
        let body = if let Some(res) = self.responses.get(url) {
            match res {
                Ok(s) => s.clone(),
                Err(PortError::NotFound) => return Err(PortError::NotFound),
                Err(PortError::Downstream { message }) => {
                    return Err(PortError::Downstream {
                        message: message.clone(),
                    });
                }
                _ => return Err(downstream_error("unknown fixture error")),
            }
        } else {
            return Err(PortError::Downstream {
                message: "connection refused".to_string(),
            });
        };
        if body.len() > max_bytes {
            return Err(PortError::InvalidInput {
                message: "web response exceeds max_bytes".to_string(),
            });
        }
        Ok(HttpResponse {
            body,
            content_type: Some("text/html".to_string()),
        })
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

    let data = fetcher.fetch(url, 1024)?;
    assert_eq!(data.url, url);
    assert_eq!(data.html, "<html><body>hello</body></html>");
    assert_eq!(data.content_hash, content_hash(data.html.as_bytes()));
    assert_eq!(data.metadata.content_type.as_deref(), Some("text/html"));
    assert!(!data.metadata.is_dynamic);
    assert!(!data.metadata.is_paywalled);
    Ok(())
}

#[test]
fn test_fetch_extracts_web_boundary_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let url = "https://example.com/research";
    let html = r#"<html><head>
        <meta property="article:published_time" content="2026-07-16">
        <meta property="article:modified_time" content="2026-07-17">
    </head><body><div id="__NEXT_DATA__">subscribe to continue</div></body></html>"#;
    let mut responses = BTreeMap::new();
    responses.insert(url.to_string(), Ok(html.to_string()));
    let fetcher =
        UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)));

    let data = fetcher.fetch(url, 4096)?;

    assert_eq!(data.metadata.published_at.as_deref(), Some("2026-07-16"));
    assert_eq!(data.metadata.updated_at.as_deref(), Some("2026-07-17"));
    assert!(data.metadata.is_dynamic);
    assert!(data.metadata.is_paywalled);
    assert_eq!(data.content_hash, content_hash(html.as_bytes()));
    Ok(())
}

#[test]
fn test_fetch_extracts_metadata_with_html_attribute_variants()
-> Result<(), Box<dyn std::error::Error>> {
    let url = "https://example.com/variants";
    let html = r#"<META content='2026-07-20' PROPERTY = 'ARTICLE:PUBLISHED_TIME'>
        <meta CONTENT="2026-07-21" name = 'dateModified'>
        <meta name='dateEffective' content='2026-07-22'>"#;
    let mut responses = BTreeMap::new();
    responses.insert(url.to_string(), Ok(html.to_string()));
    let fetcher =
        UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)));

    let data = fetcher.fetch(url, 4096)?;

    assert_eq!(data.metadata.published_at.as_deref(), Some("2026-07-20"));
    assert_eq!(data.metadata.updated_at.as_deref(), Some("2026-07-21"));
    assert_eq!(data.metadata.effective_at.as_deref(), Some("2026-07-22"));
    Ok(())
}

#[test]
fn test_fetch_extracts_unquoted_metadata_and_trusted_primary_source()
-> Result<(), Box<dyn std::error::Error>> {
    let url = "https://example.com/primary";
    let html = "<meta name=datePublished content=2026-07-23>";
    let mut responses = BTreeMap::new();
    responses.insert(url.to_string(), Ok(html.to_string()));
    let fetcher =
        UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)))
            .with_primary_domains(vec!["example.com".to_string()]);

    let data = fetcher.fetch(url, 4096)?;

    assert_eq!(data.metadata.published_at.as_deref(), Some("2026-07-23"));
    assert!(data.metadata.primary_source);
    Ok(())
}

#[test]
fn test_fetch_not_found() -> Result<(), Box<dyn std::error::Error>> {
    let url = "http://example.com/test-404";
    let mut responses = BTreeMap::new();
    responses.insert(url.to_string(), Err(PortError::NotFound));
    let fetcher =
        UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(responses)));

    let err = fetcher.fetch(url, 4096);
    assert!(matches!(err, Err(PortError::NotFound)));
    Ok(())
}

#[test]
fn test_fetch_invalid_url() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher =
        UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(BTreeMap::new())));

    let invalid_urls = vec![
        "",
        "ftp://example.com",
        "http://",
        "https://",
        "http:// ",
        "http://\n",
        "not-a-url",
        "http://localhost",
        "http://127.0.0.1",
        "http://169.254.169.254",
        "http://[::ffff:127.0.0.1]",
        "http://100.64.0.1",
        "http://198.18.0.1",
        "http://user:password@example.com",
    ];

    for url in invalid_urls {
        assert!(
            matches!(
                fetcher.fetch(url, usize::MAX),
                Err(PortError::InvalidInput { .. })
            ),
            "Expected InvalidInput for url: '{url}'"
        );
    }
    Ok(())
}

#[test]
fn test_fetch_connection_refused() -> Result<(), Box<dyn std::error::Error>> {
    let fetcher =
        UreqWebFetcher::with_transport(std::sync::Arc::new(FixtureTransport::new(BTreeMap::new())));
    let err = fetcher.fetch("http://example.invalid:12345", 4096);
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
