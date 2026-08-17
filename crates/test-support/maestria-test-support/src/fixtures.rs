//! Deterministic content-hash and realm-id fixture generators.
//!
//! Every generator derives its repeated hex digit from a single seed byte, so
//! callers write `content_hash(0)` instead of embedding a 64-character
//! literal. Seed values map onto `"0123456789abcdef"` by index.

use maestria_domain::{ContentHash, RealmId, RealmIdError, SearchCompatibilityError};

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Builds a `sha256:`-prefixed 64-hex-digit string whose digit is seeded.
fn hex64(seed: u8) -> String {
    format!(
        "sha256:{}",
        (HEX_DIGITS[(seed % 16) as usize] as char)
            .to_string()
            .repeat(64)
    )
}

/// Builds the bare 64-hex-digit form used by [`RealmId`].
fn realm_hex64(seed: u8) -> String {
    (HEX_DIGITS[(seed % 16) as usize] as char)
        .to_string()
        .repeat(64)
}

/// A valid content hash whose digest digit is seeded (0..=15 → `0`..=`f`).
pub fn content_hash(seed: u8) -> Result<ContentHash, SearchCompatibilityError> {
    ContentHash::new(hex64(seed))
}

/// The `sha256:`-prefixed string form of a seeded content hash.
pub fn content_hash_str(seed: u8) -> String {
    hex64(seed)
}

/// A valid realm identity whose hex digit is seeded (0..=15 → `0`..=`f`).
pub fn realm_id(seed: u8) -> Result<RealmId, RealmIdError> {
    RealmId::try_from(realm_hex64(seed))
}

/// The bare 64-hex-digit string form of a seeded realm identity.
pub fn realm_id_str(seed: u8) -> String {
    realm_hex64(seed)
}
