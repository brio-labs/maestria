use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Stable, instance-owned realm identity.
///
/// Realm identities are canonical lower-case SHA-256-sized hexadecimal values.
/// Validation happens at every serialization boundary so malformed identities
/// cannot enter manifests, grants, events, or persisted projections.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RealmId(String);

impl RealmId {
    pub fn try_from(value: String) -> Result<Self, RealmIdError> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(RealmIdError::InvalidFormat);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RealmId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("RealmId").field(&self.0).finish()
    }
}

impl fmt::Display for RealmId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmIdError {
    InvalidFormat,
}

impl fmt::Display for RealmIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("realm ID must be 64 lowercase hexadecimal characters")
    }
}

impl std::error::Error for RealmIdError {}

impl Serialize for RealmId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RealmId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_canonical_lowercase_hex_boundary() -> Result<(), RealmIdError> {
        let valid = "a".repeat(64);
        assert_eq!(RealmId::try_from(valid.clone())?.as_str(), valid);
        for invalid in ["", &"A".repeat(64), &"g".repeat(64), &"a".repeat(63)] {
            assert_eq!(
                RealmId::try_from(invalid.to_string()),
                Err(RealmIdError::InvalidFormat)
            );
        }
        Ok(())
    }

    #[test]
    fn serde_rejects_malformed_realm_id() {
        let malformed = format!("\"{}\"", "A".repeat(64));
        assert!(serde_json::from_str::<RealmId>(&malformed).is_err());
    }
}
