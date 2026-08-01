//! Single source of truth for which paths the workspace ingests.
//!
//! The CLI's index-collection policy and the daemon watcher's scan policy
//! were separate copies that drifted apart (R28): the CLI accepted
//! md|markdown|txt|text|rs|pdf plus a `Cargo.toml` special case, the watcher
//! accepted md|markdown|txt|rs|toml|json|yaml|yml|pdf with no `Cargo.toml`
//! rule. Both entry points delegate here.

use maestria_governance::PrivacyExclusions;
use std::path::Path;

/// Whether `path` names a source file eligible for ingestion.
///
/// Union of the CLI and watcher policies: the CLI's `Cargo.toml` special case
/// plus case-insensitive extensions md|markdown|txt|text|rs|toml|json|yaml|
/// yml|pdf.
pub fn is_supported_source_file(path: &Path) -> bool {
    if path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("md" | "markdown" | "txt" | "text" | "rs" | "toml" | "json" | "yaml" | "yml" | "pdf")
    )
}

/// Whether `path` traverses a privacy-excluded component.
///
/// The CLI's hard-coded component exclusions (`.ssh`, `.gnupg`, `node_modules`,
/// `target`, `dist`, `build`, `.env.*` prefixes) are OR-ed with the shared
/// governance privacy exclusions.
pub fn is_privacy_excluded_path(path: &Path) -> bool {
    let default_exclusions = PrivacyExclusions::default();
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        matches!(
            name.as_ref(),
            ".ssh" | ".gnupg" | "node_modules" | "target" | "dist" | "build"
        ) || name.starts_with(".env.")
    }) || default_exclusions.is_excluded(path)
}
