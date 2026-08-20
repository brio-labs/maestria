use sha2::{Digest, Sha256};
use std::fmt;

const DOMAIN_SEPARATOR: &[u8] = b"maestria.realm.read.grant.v1\0";

/// Provider-local durable key for a realm-read grant.
///
/// This is a domain-separated digest, never the bearer credential itself.
/// Raw bearer-token bytes are accepted only transiently to derive this value
/// and are not retained by the domain.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GrantTokenDigest(String);

impl GrantTokenDigest {
    pub fn try_from(value: String) -> Result<Self, GrantTokenDigestError> {
        if !crate::ids::is_lowercase_hex64(&value) {
            return Err(GrantTokenDigestError::InvalidFormat);
        }
        Ok(Self(value))
    }

    /// Derives a durable key without retaining the supplied credential.
    pub fn derive(bearer_token: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DOMAIN_SEPARATOR);
        hasher.update(bearer_token);
        let digest = hasher.finalize();
        let encoded = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        // A SHA-256 digest is always canonical lower-case 64-hex output.
        Self(encoded)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GrantTokenDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("GrantTokenDigest")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for GrantTokenDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantTokenDigestError {
    InvalidFormat,
}

impl fmt::Display for GrantTokenDigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("grant token digest must be 64 lowercase hexadecimal characters")
    }
}

impl std::error::Error for GrantTokenDigestError {}

impl serde::Serialize for GrantTokenDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for GrantTokenDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_is_deterministic_and_domain_separated() {
        let digest = GrantTokenDigest::derive(b"credential");
        assert_eq!(digest, GrantTokenDigest::derive(b"credential"));
        assert_ne!(digest.as_str(), "credential");
        assert_eq!(digest.as_str().len(), 64);
    }

    #[test]
    fn rejects_noncanonical_digests() {
        for invalid in ["", &"A".repeat(64), &"g".repeat(64), &"a".repeat(63)] {
            assert_eq!(
                GrantTokenDigest::try_from(invalid.to_string()),
                Err(GrantTokenDigestError::InvalidFormat)
            );
        }
    }
}
