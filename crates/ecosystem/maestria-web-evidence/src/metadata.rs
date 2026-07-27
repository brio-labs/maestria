use maestria_domain::WebEvidenceMetadata;

pub(crate) fn metadata_from_html(
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
