use maestria_domain::ContentHash;

/// A canonical content hash for deterministic test fixtures.
/// Format: "sha256:" followed by 64 hexadecimal characters.
pub fn test_content_hash() -> Result<ContentHash, Box<dyn std::error::Error>> {
    Ok(ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?)
}
