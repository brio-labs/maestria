use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::errors::LauncherError;

use super::quicklinks;

pub(super) const MAX_ENTRIES: usize = 256;
pub(super) const MAX_ID_BYTES: usize = 96;
pub(super) const MAX_TITLE_BYTES: usize = 256;
pub(super) const MAX_CONTENT_BYTES: usize = 16 * 1024;

static NEXT_UTILITY_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum UtilityKind {
    Quicklink,
    Snippet,
    Clipboard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UtilityEffect {
    OpenUri(String),
    Copy(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Entry {
    pub(crate) id: String,
    pub(crate) kind: UtilityKind,
    pub(crate) title: String,
    pub(crate) content: String,
    #[serde(skip)]
    normalized_title: String,
    #[serde(skip)]
    normalized_content: String,
}

impl Entry {
    pub(super) fn new(id: String, kind: UtilityKind, title: String, content: String) -> Self {
        let mut entry = Self {
            id,
            kind,
            title,
            content,
            normalized_title: String::new(),
            normalized_content: String::new(),
        };
        entry.rebuild_search_cache();
        entry
    }

    /// Matches a caller-normalized query without allocating per keystroke.
    pub(crate) fn matches(&self, normalized_query: &str) -> bool {
        normalized_query.is_empty()
            || self.normalized_title.contains(normalized_query)
            || self.normalized_content.contains(normalized_query)
    }

    pub(super) fn rebuild_search_cache(&mut self) {
        self.normalized_title = self.title.to_lowercase();
        self.normalized_content = self.content.to_lowercase();
    }

    pub(super) fn unique_id(mut is_in_use: impl FnMut(&str) -> bool) -> String {
        loop {
            let number = NEXT_UTILITY_ID.fetch_add(1, Ordering::Relaxed);
            let id = format!("utility-{number:016x}");
            if !is_in_use(&id) {
                return id;
            }
        }
    }
}

pub(super) fn validate_id(id: &str) -> Result<(), LauncherError> {
    if id.is_empty()
        || id.len() > MAX_ID_BYTES
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(LauncherError::invalid_request(
            "Utility IDs must be 1–96 ASCII letters, digits, hyphens, or underscores",
        ));
    }
    Ok(())
}

pub(super) fn validate_data(
    kind: UtilityKind,
    id: &str,
    title: &str,
    content: &str,
) -> Result<(), LauncherError> {
    validate_id(id)?;
    if title.trim().is_empty() || title.len() > MAX_TITLE_BYTES || title.contains('\0') {
        return Err(LauncherError::invalid_request(
            "Utility titles must be non-empty, at most 256 bytes, and contain no NUL",
        ));
    }
    if content.len() > MAX_CONTENT_BYTES {
        return Err(LauncherError::invalid_request(
            "Utility content exceeds the 16 KiB limit",
        ));
    }
    if content.contains('\0') {
        return Err(LauncherError::invalid_request(
            "Utility content cannot contain NUL",
        ));
    }
    match kind {
        UtilityKind::Quicklink => quicklinks::validate_template(content),
        UtilityKind::Snippet => Ok(()),
        UtilityKind::Clipboard => Err(LauncherError::invalid_request(
            "Clipboard entries are memory-only and cannot be persisted",
        )),
    }
}

pub(super) fn validate_entry(entry: &Entry) -> Result<(), LauncherError> {
    validate_data(entry.kind, &entry.id, &entry.title, &entry.content)
}
