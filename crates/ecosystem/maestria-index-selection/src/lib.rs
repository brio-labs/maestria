#![forbid(unsafe_code)]

//! Deterministic indexing choice layer: scan a root, classify every
//! directory with simple numeric rules into Recommended / Maybe / Noise,
//! and select a whitelist under three independent per-directory policy
//! switches (skip large files, skip generated dumps, skip minified
//! bundles). Shared by the CLI and the Studio.

/// Responsibility map:
/// - `policy`: per-file selection switches and their decisions.
/// - `scan`: directory walker, file policy, and per-directory features.
/// - `classify`: deterministic Recommended / Maybe / Noise rules.
/// - `candidates`: the candidate tree with per-directory policies.
/// - `profile`: persisted selection profiles.
pub mod candidates;
mod classify;
mod policy;
pub mod profile;
mod scan;
#[cfg(test)]
mod tests;

pub use candidates::{scan_candidates, CandidateDir};
pub use classify::{classify, default_policy, Class};
pub use policy::{group_by_child, is_notable_group, select_source, IndexPolicy, Selection};
pub use profile::{load_profile, save_profile, IndexSelectionProfile};
pub use scan::{
    collect_files, dir_features, is_home_root, is_privacy_excluded_path, is_supported_source_file,
    DirFeatures,
};
