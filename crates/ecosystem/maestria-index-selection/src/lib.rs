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

pub use candidates::{CandidateDir, bound_candidate_tree, scan_candidates};
pub use classify::{Class, classify, default_policy};
pub use policy::{IndexPolicy, Selection, group_by_child, is_notable_group, select_source};
pub use profile::{IndexSelectionProfile, load_profile, save_profile};
pub use scan::{
    DirFeatures, collect_files, dir_features, is_home_root, is_privacy_excluded_path,
    is_supported_source_file,
};
