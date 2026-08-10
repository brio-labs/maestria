//! Deterministic Recommended / Maybe / Noise classification rules.

use crate::policy::IndexPolicy;
use crate::scan::DirFeatures;
use std::path::Path;

/// The classification of a directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Class {
    Recommended,
    Maybe,
    Noise,
}

/// Directory components that mark machine-generated or build output with
/// high confidence. The ONLY name-based rule, and it applies only when
/// scanning the home directory itself.
const HOME_NOISE_COMPONENTS: &[&str] = &[
    ".cache",
    ".config",
    ".local",
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".venv",
    "__pycache__",
    ".git",
];

/// The default policy for a class: Recommended directories are indexed
/// with every switch off; Maybe and Noise directories get the filtered
/// defaults until the user says otherwise.
pub fn default_policy(class: Class) -> IndexPolicy {
    match class {
        Class::Recommended => IndexPolicy::everything(),
        Class::Maybe | Class::Noise => IndexPolicy::filtered(),
    }
}

/// Classify `path` from its numeric features.
///
/// Rules, evaluated in order:
/// 1. `home_root` and any path component in the noise set → `Noise`.
/// 2. Generated dump: at least 200 files, 90% one extension, mean file
///    under 64 KiB, and under 5% doc/code → `Noise`.
/// 3. Minified-heavy: at least half the files minified and 5 MiB total →
///    `Noise`.
/// 4. Doc-heavy: at least half docs and at most 500 files → `Recommended`.
/// 5. Code-heavy: at least 20% code and at most 2_000 files → `Recommended`.
/// 6. Everything else → `Maybe`.
pub fn classify(features: &DirFeatures, home_root: bool, path: &Path) -> Class {
    if home_root
        && path.components().any(|component| {
            let name = component.as_os_str().to_string_lossy();
            HOME_NOISE_COMPONENTS.iter().any(|marker| name == *marker)
        })
    {
        return Class::Noise;
    }
    if features.file_count >= 200
        && features.single_ext_share >= 0.90
        && features.mean_bytes < 64 * 1024
        && features.doc_share + features.code_share < 0.05
    {
        return Class::Noise;
    }
    if features.minified_share >= 0.50 && features.total_bytes >= 5 * 1024 * 1024 {
        return Class::Noise;
    }
    if features.doc_share >= 0.50 && features.file_count <= 500 {
        return Class::Recommended;
    }
    if features.code_share >= 0.20 && features.file_count <= 2_000 {
        return Class::Recommended;
    }
    Class::Maybe
}
