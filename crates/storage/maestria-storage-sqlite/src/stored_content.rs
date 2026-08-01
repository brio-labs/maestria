use maestria_domain::ContentHash;
use serde::{Deserialize, Serialize};

/// Wire mirror of `maestria_domain::ContentHash`. The domain type serializes
/// as a plain string (`#[serde(try_from = "String")]`), so the stored row
/// keeps the raw string and validates on decode via `ContentHash::new`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredContentHash(pub(crate) String);

impl StoredContentHash {
    pub(crate) fn from_domain(value: &ContentHash) -> Self {
        Self(value.as_str().to_owned())
    }

    pub(crate) fn try_into_domain(self) -> Result<ContentHash, maestria_ports::PortError> {
        ContentHash::new(self.0).map_err(|error| maestria_ports::PortError::InvalidInputContext {
            context: "stored content hash is invalid",
            source: error.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = ContentHash::new(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        )?;
        let stored = StoredContentHash::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn content_hash_rejects_invalid_string() {
        let stored = StoredContentHash("sha256:not-a-digest".to_owned());
        let result = stored.try_into_domain();
        assert!(matches!(
            result,
            Err(maestria_ports::PortError::InvalidInputContext {
                context: "stored content hash is invalid",
                ..
            })
        ));
    }

    #[test]
    fn content_hash_rejects_uppercase_hex() {
        let stored = StoredContentHash(
            "sha256:0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF".to_owned(),
        );
        assert!(stored.try_into_domain().is_err());
    }

    #[test]
    fn content_hash_serializes_as_plain_string() -> Result<(), Box<dyn std::error::Error>> {
        let stored = StoredContentHash::from_domain(&ContentHash::new(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
        )?);
        let json = serde_json::to_string(&stored)?;
        assert_eq!(
            json,
            "\"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\""
        );
        let decoded: StoredContentHash = serde_json::from_str(&json)?;
        assert_eq!(decoded, stored);
        Ok(())
    }
}
