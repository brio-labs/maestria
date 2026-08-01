//! Runtime blocked-pattern composition.
//!
//! Runtime construction and the read API each composed the runtime blocked
//! patterns (manifest exclusions + privacy-sensitive names + `*.ext` forms of
//! privacy-sensitive extensions) in their own copy (R28). This module is the
//! single source of truth for the composition.

use maestria_core::InstanceManifest;
use maestria_governance::PrivacyExclusions;

/// Compose the runtime blocked patterns for `manifest`: manifest exclusions,
/// privacy-sensitive names, and `*.ext` forms of privacy-sensitive extensions.
pub fn runtime_blocked_patterns(manifest: &InstanceManifest) -> Vec<String> {
    let default_privacy = PrivacyExclusions::default();
    let mut blocked_patterns = manifest.excluded_patterns.clone();
    blocked_patterns.extend(default_privacy.sensitive_names().iter().cloned());
    blocked_patterns.extend(
        default_privacy
            .sensitive_extensions()
            .iter()
            .map(|extension| format!("*.{extension}")),
    );
    blocked_patterns
}
