#![forbid(unsafe_code)]

//! Synchronous web evidence adapter backed by `ureq`.
//!
//! The adapter validates URL schemes, hashes fetched bytes, and extracts only
//! source metadata from HTML. Runtime orchestration owns blob persistence,
//! security scanning, policy decisions, and domain evidence recording.

use maestria_domain::{WebEvidenceMetadata, content_hash};
use maestria_ports::{PortError, WebFetchOptions, WebFetcher, WebSnapshotData};
use std::collections::BTreeSet;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;
const MAX_WEB_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
struct HttpResponse {
    body: String,
    content_type: Option<String>,
}

trait HttpTransport: Send + Sync + std::fmt::Debug {
    fn get(&self, url: &str, max_bytes: usize) -> Result<HttpResponse, PortError>;
}
fn metadata_from_html(
    html: &str,
    content_type: Option<String>,
    primary_source: bool,
) -> WebEvidenceMetadata {
    WebEvidenceMetadata {
        published_at: meta_content(html, "article:published_time")
            .or_else(|| meta_content(html, "datePublished")),
        updated_at: meta_content(html, "article:modified_time")
            .or_else(|| meta_content(html, "dateModified")),
        effective_at: meta_content(html, "dateEffective"),
        accessed_at: None,
        content_type,
        primary_source,
        is_dynamic: is_dynamic_page(html),
        is_paywalled: is_paywalled_page(html),
    }
}

fn meta_content(html: &str, name: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search_start = 0;

    while let Some(relative_start) = lower[search_start..].find("<meta") {
        let tag_start = search_start.saturating_add(relative_start);
        let Some(relative_end) = lower[tag_start..].find('>') else {
            break;
        };
        let tag_end = tag_start.saturating_add(relative_end);
        let tag = &html[tag_start..=tag_end];
        let identity = attribute_value(tag, "name").or_else(|| attribute_value(tag, "property"));
        if identity
            .as_deref()
            .is_some_and(|value| value.eq_ignore_ascii_case(name))
        {
            return attribute_value(tag, "content").and_then(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            });
        }
        search_start = tag_end.saturating_add(1);
    }

    None
}

fn attribute_value(tag: &str, attribute: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        while index < bytes.len()
            && !(bytes[index].is_ascii_alphabetic() || bytes[index] == b':' || bytes[index] == b'_')
        {
            index = index.saturating_add(1);
        }
        let name_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric()
                || bytes[index] == b':'
                || bytes[index] == b'_'
                || bytes[index] == b'-')
        {
            index = index.saturating_add(1);
        }
        let name_end = index;
        if name_start == index {
            continue;
        }
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index = index.saturating_add(1);
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index = index.saturating_add(1);
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index = index.saturating_add(1);
        }
        if index >= bytes.len() {
            continue;
        }
        let (value_start, value_end, next_index) = if matches!(bytes[index], b'\'' | b'"') {
            let quote = bytes[index];
            let value_start = index.saturating_add(1);
            let value_length = bytes[value_start..]
                .iter()
                .position(|byte| *byte == quote)?;
            let value_end = value_start.saturating_add(value_length);
            (value_start, value_end, value_end.saturating_add(1))
        } else {
            let value_start = index;
            let value_length = match bytes[value_start..]
                .iter()
                .position(|byte| byte.is_ascii_whitespace() || *byte == b'>')
            {
                Some(length) => length,
                None => {
                    let _ = ();
                    bytes.len().saturating_sub(value_start)
                }
            };
            let value_end = value_start.saturating_add(value_length);
            (value_start, value_end, value_end)
        };
        let name = &tag[name_start..name_end];
        if name.eq_ignore_ascii_case(attribute) {
            return Some(tag[value_start..value_end].to_string());
        }
        index = next_index;
    }

    None
}

fn is_dynamic_page(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("__next_data__")
        || lower.contains("data-reactroot")
        || (lower.contains("<script") && lower.contains("enable javascript"))
}

fn is_paywalled_page(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    [
        "subscribe to continue",
        "subscription required",
        "sign in to read",
        "paywall",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}
fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            let is_cgnat = octets[0] == 100 && (64..=127).contains(&octets[1]);
            let is_documentation_or_benchmark =
                (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                    || (octets[0] == 198 && (18..=19).contains(&octets[1]));
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || octets[0] == 0
                || is_cgnat
                || is_documentation_or_benchmark
        }
        IpAddr::V6(ip) => {
            ip.to_ipv4()
                .is_some_and(|mapped| blocked_ip(IpAddr::V4(mapped)))
                || ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

fn domain_allowed(parsed: &url::Url, allowed_domains: &[String]) -> bool {
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowed_domains.iter().any(|domain| {
        let domain = domain.trim_end_matches('.').to_ascii_lowercase();
        host == domain
            || host
                .strip_suffix(&domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn resolve_public(netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
    let addresses: Vec<_> = netloc.to_socket_addrs()?.collect();
    if addresses.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "host did not resolve to an address",
        ));
    }
    if addresses.iter().any(|address| blocked_ip(address.ip())) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "private or local web hosts are not allowed",
        ));
    }
    Ok(addresses)
}

fn validate_fetch_url(parsed: &url::Url) -> Result<(), PortError> {
    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(PortError::InvalidInput {
            message: "url must have http or https scheme".to_string(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(PortError::InvalidInput {
            message: "url credentials are not allowed".to_string(),
        });
    }
    let Some(host) = parsed.host() else {
        return Err(PortError::InvalidInput {
            message: "url must have a host".to_string(),
        });
    };
    let blocked = match host {
        url::Host::Domain(host) => {
            let normalized_host = host.trim_end_matches('.').to_ascii_lowercase();
            normalized_host.is_empty()
                || normalized_host == "localhost"
                || normalized_host.ends_with(".localhost")
                || normalized_host.ends_with(".local")
        }
        url::Host::Ipv4(ip) => blocked_ip(IpAddr::V4(ip)),
        url::Host::Ipv6(ip) => blocked_ip(IpAddr::V6(ip)),
    };
    if blocked {
        return Err(PortError::InvalidInput {
            message: "private or local web hosts are not allowed".to_string(),
        });
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct UreqTransport {
    agent: ureq::Agent,
}

impl HttpTransport for UreqTransport {
    fn get(&self, url: &str, max_bytes: usize) -> Result<HttpResponse, PortError> {
        if max_bytes == 0 || max_bytes > MAX_WEB_RESPONSE_BYTES {
            return Err(PortError::InvalidInput {
                message: format!("max_bytes must be between 1 and {MAX_WEB_RESPONSE_BYTES}"),
            });
        }
        let response = match self.agent.get(url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(404, _)) => return Err(PortError::NotFound),
            Err(e) => return Err(downstream_error(e)),
        };
        if (300..400).contains(&response.status()) {
            return Err(PortError::InvalidInput {
                message: "web redirects are not allowed".to_string(),
            });
        }
        let content_type = response.header("content-type").map(str::to_owned);
        let read_limit = u64::try_from(max_bytes).map_or(u64::MAX, |value| value.saturating_add(1));
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(downstream_error)?;
        if bytes.len() > max_bytes {
            return Err(PortError::InvalidInput {
                message: "web response exceeds max_bytes".to_string(),
            });
        }
        let body = String::from_utf8(bytes).map_err(downstream_error)?;
        Ok(HttpResponse { body, content_type })
    }
}

#[derive(Clone, Debug)]
pub struct UreqWebFetcher {
    transport: std::sync::Arc<dyn HttpTransport>,
    primary_domains: BTreeSet<String>,
}

impl Default for UreqWebFetcher {
    fn default() -> Self {
        Self {
            transport: std::sync::Arc::new(UreqTransport {
                agent: ureq::AgentBuilder::new()
                    .timeout(Duration::from_secs(15))
                    .resolver(resolve_public)
                    .redirects(0)
                    .build(),
            }),
            primary_domains: BTreeSet::new(),
        }
    }
}

impl UreqWebFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_primary_domains(mut self, domains: impl IntoIterator<Item = String>) -> Self {
        self.primary_domains = domains
            .into_iter()
            .map(|domain| domain.trim_end_matches('.').to_ascii_lowercase())
            .filter(|domain| !domain.is_empty())
            .collect();
        self
    }

    #[cfg(test)]
    fn with_transport(transport: std::sync::Arc<dyn HttpTransport>) -> Self {
        Self {
            transport,
            primary_domains: BTreeSet::new(),
        }
    }
}

impl WebFetcher for UreqWebFetcher {
    fn fetch(&self, url_str: &str, max_bytes: usize) -> Result<WebSnapshotData, PortError> {
        self.fetch_with_options(url_str, &WebFetchOptions::new(max_bytes))
    }

    fn fetch_with_options(
        &self,
        url_str: &str,
        options: &WebFetchOptions,
    ) -> Result<WebSnapshotData, PortError> {
        if options.max_bytes == 0 || options.max_bytes > MAX_WEB_RESPONSE_BYTES {
            return Err(PortError::InvalidInput {
                message: format!("max_bytes must be between 1 and {MAX_WEB_RESPONSE_BYTES}"),
            });
        }
        if options.max_latency_ms == 0 {
            return Err(PortError::InvalidInput {
                message: "max_latency_ms must be greater than zero".to_string(),
            });
        }
        let parsed = url::Url::parse(url_str).map_err(|e| PortError::InvalidInput {
            message: format!("invalid url: {e}"),
        })?;
        validate_fetch_url(&parsed)?;
        if !options.allowed_domains.is_empty() && !domain_allowed(&parsed, &options.allowed_domains)
        {
            return Err(PortError::InvalidInput {
                message: "url is outside the allowed web domains".to_string(),
            });
        }
        let response = self.transport.get(url_str, options.max_bytes)?;
        if !options.allowed_content_types.is_empty()
            && !response
                .content_type
                .as_deref()
                .is_some_and(|content_type| {
                    options
                        .allowed_content_types
                        .iter()
                        .any(|allowed| content_type.starts_with(allowed))
                })
        {
            return Err(PortError::InvalidInput {
                message: "web response content type is not allowed".to_string(),
            });
        }
        let primary_source = parsed.host_str().is_some_and(|host| {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            self.primary_domains.iter().any(|domain| {
                host == *domain
                    || host
                        .strip_suffix(domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            })
        });
        let metadata = metadata_from_html(&response.body, response.content_type, primary_source);
        Ok(WebSnapshotData {
            url: url_str.to_string(),
            content_hash: content_hash(response.body.as_bytes()),
            html: response.body,
            metadata,
        })
    }
}

fn downstream_error(error: impl std::fmt::Display) -> PortError {
    PortError::Downstream {
        message: error.to_string(),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
