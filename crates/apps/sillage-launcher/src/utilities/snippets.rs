use crate::errors::LauncherError;

use super::MAX_EXPANSION_BYTES;

const QUERY_PLACEHOLDER: &str = "{query}";

pub(super) fn expand(template: &str, argument: &str) -> Result<String, LauncherError> {
    let count = template.matches(QUERY_PLACEHOLDER).count();
    let removed_bytes = count
        .checked_mul(QUERY_PLACEHOLDER.len())
        .ok_or_else(expansion_too_large)?;
    let output_bytes = template
        .len()
        .checked_sub(removed_bytes)
        .and_then(|length| length.checked_add(count.checked_mul(argument.len())?))
        .filter(|length| *length <= MAX_EXPANSION_BYTES)
        .ok_or_else(expansion_too_large)?;

    let mut expanded = String::with_capacity(output_bytes);
    let mut remainder = template;
    while let Some(index) = remainder.find(QUERY_PLACEHOLDER) {
        expanded.push_str(&remainder[..index]);
        expanded.push_str(argument);
        remainder = &remainder[index + QUERY_PLACEHOLDER.len()..];
    }
    expanded.push_str(remainder);
    Ok(expanded)
}

fn expansion_too_large() -> LauncherError {
    LauncherError::invalid_request("Utility expansion exceeds the 64 KiB limit")
}
