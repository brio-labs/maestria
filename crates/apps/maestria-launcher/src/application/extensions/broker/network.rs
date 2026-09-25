use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use maestria_extensions::HttpMethod;
use url::{Host, Url};

use super::text::decode_bounded;

const MAX_HTTP_BODY_BYTES: usize = 16_384;
const MAX_DNS_ADDRESSES: usize = 64;
const DNS_TIMEOUT: Duration = Duration::from_secs(3);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub(super) enum NetworkError {
    Denied,
    InvalidRequest,
    Unavailable,
    Failed,
}

pub(super) struct NetworkResponse {
    pub(super) status: u16,
    pub(super) body: String,
    pub(super) truncated: bool,
}

pub(super) async fn request(
    raw_url: &str,
    method: HttpMethod,
    body: Option<&str>,
) -> Result<NetworkResponse, NetworkError> {
    if body.is_some_and(|value| value.len() > MAX_HTTP_BODY_BYTES)
        || (matches!(method, HttpMethod::Get) && body.is_some())
    {
        return Err(NetworkError::InvalidRequest);
    }
    let url = Url::parse(raw_url).map_err(|_| NetworkError::InvalidRequest)?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || raw_url.len() > 4096
    {
        return Err(NetworkError::InvalidRequest);
    }
    let (host, addresses) = resolve_public(&url).await?;
    let url = url.to_string();
    let body = match body {
        Some(body) => body.as_bytes().to_vec(),
        None => Vec::new(),
    };
    let request = request_pinned(url, host, addresses, method, body);
    match tokio::time::timeout(HTTP_TIMEOUT, request).await {
        Ok(result) => result,
        Err(_) => Err(NetworkError::Unavailable),
    }
}

async fn resolve_public(url: &Url) -> Result<(String, Vec<SocketAddr>), NetworkError> {
    let host = url.host().ok_or(NetworkError::InvalidRequest)?;
    if blocked_host(host) {
        return Err(NetworkError::Denied);
    }
    let name = url
        .host_str()
        .ok_or(NetworkError::InvalidRequest)?
        .to_owned();
    let port = url
        .port_or_known_default()
        .ok_or(NetworkError::InvalidRequest)?;
    let lookup = tokio::time::timeout(DNS_TIMEOUT, tokio::net::lookup_host((name.as_str(), port)))
        .await
        .map_err(|_| NetworkError::Unavailable)?
        .map_err(|_| NetworkError::Unavailable)?;
    let addresses = lookup.take(MAX_DNS_ADDRESSES + 1).collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(NetworkError::Unavailable);
    }
    if addresses.len() > MAX_DNS_ADDRESSES
        || addresses.iter().any(|address| blocked_ip(address.ip()))
    {
        return Err(NetworkError::Denied);
    }
    Ok((name, addresses))
}

async fn request_pinned(
    url: String,
    host: String,
    addresses: Vec<SocketAddr>,
    method: HttpMethod,
    body: Vec<u8>,
) -> Result<NetworkResponse, NetworkError> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(3))
        .read_timeout(Duration::from_secs(5))
        .timeout(HTTP_TIMEOUT)
        .http2_max_header_list_size(32_768)
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|_| NetworkError::Unavailable)?;
    let request = match method {
        HttpMethod::Get => client.get(&url),
        HttpMethod::Post => client.post(&url).body(body),
    };
    let mut response = request
        .send()
        .await
        .map_err(|_| NetworkError::Unavailable)?;
    if response_header_bytes(&response) > 32_768 {
        return Err(NetworkError::Failed);
    }
    let status = response.status().as_u16();
    let mut bytes = Vec::with_capacity(MAX_HTTP_BODY_BYTES + 1);
    while bytes.len() <= MAX_HTTP_BODY_BYTES {
        let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| NetworkError::Unavailable)?
        else {
            break;
        };
        let remaining = MAX_HTTP_BODY_BYTES + 1 - bytes.len();
        bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if bytes.len() > MAX_HTTP_BODY_BYTES {
            break;
        }
    }
    let (body, truncated) =
        decode_bounded(bytes, MAX_HTTP_BODY_BYTES, false).map_err(|_| NetworkError::Failed)?;
    Ok(NetworkResponse {
        status,
        body,
        truncated,
    })
}

fn response_header_bytes(response: &reqwest::Response) -> usize {
    response
        .headers()
        .iter()
        .fold(0usize, |total, (name, value)| {
            total
                .saturating_add(name.as_str().len())
                .saturating_add(value.as_bytes().len())
                .saturating_add(4)
        })
}

fn blocked_host(host: Host<&str>) -> bool {
    match host {
        Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            domain == "localhost"
                || domain.ends_with(".localhost")
                || domain.ends_with(".local")
                || domain.ends_with(".internal")
        }
        Host::Ipv4(address) => blocked_ip(IpAddr::V4(address)),
        Host::Ipv6(address) => blocked_ip(IpAddr::V6(address)),
    }
}

fn blocked_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            let carrier_nat = octets[0] == 100 && (64..=127).contains(&octets[1]);
            let documentation = (octets[0] == 192 && octets[1] == 0)
                || (octets[0] == 198 && matches!(octets[1], 18 | 19 | 51))
                || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113);
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_multicast()
                || octets[0] == 0
                || octets[0] >= 240
                || carrier_nat
                || documentation
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            address
                .to_ipv4()
                .is_some_and(|mapped| blocked_ip(IpAddr::V4(mapped)))
                || address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || (segments[0] == 0x2002)
                || (segments[0] == 0x0064 && segments[1] == 0xff9b)
        }
    }
}
