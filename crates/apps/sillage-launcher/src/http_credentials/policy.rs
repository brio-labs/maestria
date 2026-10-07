use sillage_extensions::HttpMethod;
use url::Url;

use super::{HttpGrantError, HttpGrantPolicy};

const MAX_LABEL_BYTES: usize = 128;
const MAX_ORIGIN_BYTES: usize = 512;
const MAX_PATH_BYTES: usize = 2048;
const MAX_TTL_SECONDS: u64 = 31_536_000;
pub(super) const MAX_SECRET_BYTES: usize = 4096;

pub(super) fn validate_policy(policy: &HttpGrantPolicy) -> Result<(), HttpGrantError> {
    if !valid_label(&policy.provider_label)
        || !valid_extension_id(&policy.extension_id)
        || !valid_package_sha256(&policy.package_sha256)
        || policy.expires_in_seconds == 0
        || policy.expires_in_seconds > MAX_TTL_SECONDS
        || !canonical_origin(&policy.origin)
        || !valid_scope_path(&policy.path)
    {
        return Err(HttpGrantError::InvalidPolicy);
    }
    Ok(())
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL_BYTES
        && label.trim() == label
        && !label.chars().any(char::is_control)
}

fn valid_extension_id(id: &str) -> bool {
    id.len() <= MAX_LABEL_BYTES
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && id.split(['.', '-']).all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

fn valid_package_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_origin(origin: &str) -> bool {
    if origin.is_empty() || origin.len() > MAX_ORIGIN_BYTES {
        return false;
    }
    let Ok(parsed) = Url::parse(origin) else {
        return false;
    };
    parsed.scheme() == "https"
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed
            .as_str()
            .strip_suffix('/')
            .is_some_and(|canonical| canonical == origin)
}

pub(super) fn valid_scope_path(path: &str) -> bool {
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || !path.starts_with('/')
        || !path.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
        || path
            .chars()
            .any(|character| matches!(character, '?' | '#' | '\\'))
    {
        return false;
    }
    let bytes = path.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return false;
            }
            let (Some(high), Some(low)) =
                (hex_value(bytes[index + 1]), hex_value(bytes[index + 2]))
            else {
                return false;
            };
            let decoded = high * 16 + low;
            if matches!(decoded, b'/' | b'\\' | b'.' | b'%') {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    !path.split('/').any(|segment| matches!(segment, "." | ".."))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(super) fn request_matches(policy: &HttpGrantPolicy, url: &Url, method: HttpMethod) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && request_origin_matches(&policy.origin, url)
        && url.path() == policy.path
        && valid_scope_path(url.path())
        && same_method(method, policy.method)
}

fn request_origin_matches(policy_origin: &str, url: &Url) -> bool {
    url.as_str()
        .strip_prefix(policy_origin)
        .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?') || rest.starts_with('#'))
}

fn same_method(left: HttpMethod, right: HttpMethod) -> bool {
    matches!(
        (left, right),
        (HttpMethod::Get, HttpMethod::Get) | (HttpMethod::Post, HttpMethod::Post)
    )
}

pub(super) fn validate_secret(secret: &[u8]) -> Result<(), HttpGrantError> {
    if secret.is_empty() || secret.len() > MAX_SECRET_BYTES || !valid_bearer_token(secret) {
        return Err(HttpGrantError::InvalidSecret);
    }
    Ok(())
}

fn valid_bearer_token(secret: &[u8]) -> bool {
    let mut padding = false;
    let mut token_bytes = 0usize;
    for byte in secret {
        if *byte == b'=' {
            padding = true;
            continue;
        }
        if padding
            || !(byte.is_ascii_alphanumeric()
                || matches!(*byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/'))
        {
            return false;
        }
        token_bytes += 1;
    }
    token_bytes != 0
}
