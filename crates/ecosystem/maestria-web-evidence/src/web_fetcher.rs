use maestria_domain::content_hash;
use maestria_ports::{PortError, WebFetchOptions, WebFetcher, WebSnapshotData};
use std::collections::BTreeSet;
use std::io::Read;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

const MAX_WEB_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct HttpResponse {
    pub(crate) body: String,
    pub(crate) content_type: Option<String>,
}

#[path = "metadata.rs"]
mod metadata;

pub(super) trait HttpTransport: Send + Sync + std::fmt::Debug {
    fn get(&self, url: &str, max_bytes: usize) -> Result<HttpResponse, PortError>;
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
        return Err(PortError::InvalidInputContext {
            context: "validate web fetch URL scheme",
            source: "url must have http or https scheme".to_string(),
        });
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(PortError::InvalidInputContext {
            context: "validate web fetch URL credentials",
            source: "url credentials are not allowed".to_string(),
        });
    }
    let Some(host) = parsed.host() else {
        return Err(PortError::InvalidInputContext {
            context: "validate web fetch URL host",
            source: "url must have a host".to_string(),
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
        return Err(PortError::InvalidInputContext {
            context: "validate web fetch URL host safety",
            source: "private or local web hosts are not allowed".to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "max_bytes out of bounds",
                source: max_bytes.to_string(),
            });
        }
        let response = match self.agent.get(url).call() {
            Ok(resp) => resp,
            Err(ureq::Error::Status(404, _)) => return Err(PortError::NotFound),
            Err(e) => return Err(downstream_error(e)),
        };
        if (300..400).contains(&response.status()) {
            return Err(PortError::InvalidInputContext {
                context: "validate web response redirect",
                source: "web redirects are not allowed".to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "validate web response size",
                source: "web response exceeds max_bytes".to_string(),
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
    pub(crate) fn with_transport(transport: std::sync::Arc<dyn HttpTransport>) -> Self {
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
            return Err(PortError::InvalidInputContext {
                context: "max_bytes out of bounds",
                source: options.max_bytes.to_string(),
            });
        }
        if options.max_latency_ms == 0 {
            return Err(PortError::InvalidInputContext {
                context: "validate web fetch latency",
                source: "max_latency_ms must be greater than zero".to_string(),
            });
        }
        let parsed = url::Url::parse(url_str).map_err(|e| PortError::InvalidInputContext {
            context: "invalid url",
            source: e.to_string(),
        })?;
        validate_fetch_url(&parsed)?;
        if !options.allowed_domains.is_empty() && !domain_allowed(&parsed, &options.allowed_domains)
        {
            return Err(PortError::InvalidInputContext {
                context: "validate web fetch domain",
                source: "url is outside the allowed web domains".to_string(),
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
            return Err(PortError::InvalidInputContext {
                context: "validate web response content type",
                source: "web response content type is not allowed".to_string(),
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
        let metadata =
            metadata::metadata_from_html(&response.body, response.content_type, primary_source);
        Ok(WebSnapshotData {
            url: url_str.to_string(),
            content_hash: content_hash(response.body.as_bytes()),
            html: response.body,
            metadata,
        })
    }
}

pub(super) fn downstream_error(error: impl std::fmt::Display) -> PortError {
    PortError::DownstreamContext {
        context: "web response error",
        source: error.to_string(),
    }
}
