use url::Url;

use crate::errors::LauncherError;

use super::MAX_EXPANSION_BYTES;

pub(super) const MAX_ARGUMENT_BYTES: usize = 16 * 1024;
const QUERY_PLACEHOLDER: &str = "{query}";

pub(super) fn validate_template(template: &str) -> Result<(), LauncherError> {
    if template.is_empty() {
        return Err(LauncherError::invalid_request(
            "Quicklink templates must be non-empty",
        ));
    }
    let placeholder_count = query_placeholder_count(template)?;
    if placeholder_count > 1 {
        return Err(LauncherError::invalid_request(
            "Quicklink templates may contain {query} at most once",
        ));
    }
    if url_authority(template).is_some_and(|authority| authority.contains(QUERY_PLACEHOLDER)) {
        return Err(LauncherError::invalid_request(
            "Quicklink {query} cannot appear in the URL authority",
        ));
    }

    if placeholder_count == 0 {
        validate_http_url(template)?;
    } else {
        let (prefix, suffix) = template.split_once(QUERY_PLACEHOLDER).ok_or_else(|| {
            LauncherError::invalid_request("Quicklink template has an invalid {query} placeholder")
        })?;
        let mut probe = String::with_capacity(prefix.len() + "sillage-query".len() + suffix.len());
        probe.push_str(prefix);
        probe.push_str("sillage-query");
        probe.push_str(suffix);
        validate_http_url(&probe)?;
    }
    Ok(())
}

pub(super) fn expand(template: &str, argument: &str) -> Result<String, LauncherError> {
    let placeholder_count = query_placeholder_count(template)?;
    if placeholder_count == 0 {
        return Ok(validate_http_url(template)?.to_string());
    }
    if placeholder_count > 1 {
        return Err(LauncherError::invalid_request(
            "Quicklink templates may contain {query} at most once",
        ));
    }
    if argument.len() > MAX_ARGUMENT_BYTES {
        return Err(LauncherError::invalid_request(
            "Expansion argument exceeds the 16 KiB limit",
        ));
    }

    let (prefix, suffix) = template.split_once(QUERY_PLACEHOLDER).ok_or_else(|| {
        LauncherError::invalid_request("Quicklink template is missing its {query} placeholder")
    })?;
    let output_bytes = prefix
        .len()
        .checked_add(encoded_uri_component_len(argument))
        .and_then(|length| length.checked_add(suffix.len()))
        .filter(|length| *length <= MAX_EXPANSION_BYTES)
        .ok_or_else(|| {
            LauncherError::invalid_request("Utility expansion exceeds the 64 KiB limit")
        })?;
    let mut expanded = String::with_capacity(output_bytes);
    expanded.push_str(prefix);
    append_uri_component(&mut expanded, argument);
    expanded.push_str(suffix);
    Ok(validate_http_url(&expanded)?.to_string())
}

fn query_placeholder_count(template: &str) -> Result<usize, LauncherError> {
    let bytes = template.as_bytes();
    let mut index = 0;
    let mut count = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'{' if bytes[index..].starts_with(QUERY_PLACEHOLDER.as_bytes()) => {
                count += 1;
                index += QUERY_PLACEHOLDER.len();
            }
            b'{' | b'}' => {
                return Err(LauncherError::invalid_request(
                    "Quicklink templates support only the {query} placeholder",
                ));
            }
            _ => index += 1,
        }
    }
    Ok(count)
}

fn url_authority(template: &str) -> Option<&str> {
    let separator = template.find("://")?;
    let authority_start = separator + 3;
    let remainder = template.get(authority_start..)?;
    let authority_end = match remainder.find(['/', '?', '#']) {
        Some(index) => index,
        None => remainder.len(),
    };
    remainder.get(..authority_end)
}

fn validate_http_url(value: &str) -> Result<Url, LauncherError> {
    let url = Url::parse(value).map_err(|_| {
        LauncherError::invalid_request("Quicklinks must be valid absolute HTTP or HTTPS URLs")
    })?;
    let authority_has_at = url_authority(value).is_some_and(|authority| authority.contains('@'));
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || authority_has_at
    {
        return Err(LauncherError::invalid_request(
            "Quicklinks must use HTTP or HTTPS and cannot contain URL credentials",
        ));
    }
    Ok(url)
}

fn encoded_uri_component_len(value: &str) -> usize {
    value.bytes().fold(0usize, |length, byte| {
        length
            + if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                1
            } else {
                3
            }
    })
}

fn append_uri_component(encoded: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
}
