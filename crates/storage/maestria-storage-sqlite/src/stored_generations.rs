use crate::payloads::stored_content::StoredContentHash;
use maestria_domain::{IndexFingerprint, IndexLifecycle, RepresentationName};
use serde::{Deserialize, Serialize};

/// Wire mirror of `maestria_domain::RepresentationName`. The domain newtype
/// serializes as a plain string, so the stored row keeps the raw value and
/// rebuilds it with the (infallible) domain constructor on decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StoredRepresentationName(pub(crate) String);

impl StoredRepresentationName {
    pub(crate) fn from_domain(name: &RepresentationName) -> Self {
        Self(name.0.clone())
    }

    pub(crate) fn try_into_domain(self) -> Result<RepresentationName, maestria_ports::PortError> {
        Ok(RepresentationName::new(self.0))
    }
}

/// Wire mirror of `maestria_domain::IndexFingerprint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredIndexFingerprint {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) revision: String,
    pub(crate) artifact_hash: StoredContentHash,
    pub(crate) dimensions: u32,
    pub(crate) quantization: String,
    pub(crate) query_template_hash: String,
    pub(crate) document_template_hash: String,
    pub(crate) preprocessing_version: String,
}

impl StoredIndexFingerprint {
    pub(crate) fn from_domain(fingerprint: &IndexFingerprint) -> Self {
        Self {
            provider: fingerprint.provider.clone(),
            model: fingerprint.model.clone(),
            revision: fingerprint.revision.clone(),
            artifact_hash: StoredContentHash::from_domain(&fingerprint.artifact_hash),
            dimensions: fingerprint.dimensions,
            quantization: fingerprint.quantization.clone(),
            query_template_hash: fingerprint.query_template_hash.clone(),
            document_template_hash: fingerprint.document_template_hash.clone(),
            preprocessing_version: fingerprint.preprocessing_version.clone(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<IndexFingerprint, maestria_ports::PortError> {
        Ok(IndexFingerprint {
            provider: self.provider,
            model: self.model,
            revision: self.revision,
            artifact_hash: self.artifact_hash.try_into_domain()?,
            dimensions: self.dimensions,
            quantization: self.quantization,
            query_template_hash: self.query_template_hash,
            document_template_hash: self.document_template_hash,
            preprocessing_version: self.preprocessing_version,
        })
    }
}

/// Wire mirror of `maestria_domain::IndexLifecycle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredIndexLifecycle {
    Building,
    Evaluated,
    Shadow,
    Active,
    Retired,
    Collectable,
    Tombstoned,
}

impl StoredIndexLifecycle {
    pub(crate) fn from_domain(lifecycle: IndexLifecycle) -> Self {
        match lifecycle {
            IndexLifecycle::Building => Self::Building,
            IndexLifecycle::Evaluated => Self::Evaluated,
            IndexLifecycle::Shadow => Self::Shadow,
            IndexLifecycle::Active => Self::Active,
            IndexLifecycle::Retired => Self::Retired,
            IndexLifecycle::Collectable => Self::Collectable,
            IndexLifecycle::Tombstoned => Self::Tombstoned,
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<IndexLifecycle, maestria_ports::PortError> {
        Ok(match self {
            Self::Building => IndexLifecycle::Building,
            Self::Evaluated => IndexLifecycle::Evaluated,
            Self::Shadow => IndexLifecycle::Shadow,
            Self::Active => IndexLifecycle::Active,
            Self::Retired => IndexLifecycle::Retired,
            Self::Collectable => IndexLifecycle::Collectable,
            Self::Tombstoned => IndexLifecycle::Tombstoned,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::ContentHash;

    const VALID_HASH: &str =
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn fingerprint() -> Result<IndexFingerprint, Box<dyn std::error::Error>> {
        Ok(IndexFingerprint {
            provider: "test-provider".to_string(),
            model: "test-model".to_string(),
            revision: "rev-1".to_string(),
            artifact_hash: ContentHash::new(VALID_HASH.to_string())?,
            dimensions: 768,
            quantization: "int8".to_string(),
            query_template_hash: "q-template".to_string(),
            document_template_hash: "d-template".to_string(),
            preprocessing_version: "v1".to_string(),
        })
    }

    #[test]
    fn representation_name_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = RepresentationName::new("learned-sparse-v2");
        let stored = StoredRepresentationName::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn fingerprint_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = fingerprint()?;
        let stored = StoredIndexFingerprint::from_domain(&original);
        let json = serde_json::to_string(&stored)?;
        let decoded = serde_json::from_str::<StoredIndexFingerprint>(&json)?;
        let restored = decoded.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn invalid_artifact_hash_fails_domain_decode() -> Result<(), Box<dyn std::error::Error>> {
        let stored = StoredIndexFingerprint {
            artifact_hash: StoredContentHash("not-a-sha256".to_string()),
            ..StoredIndexFingerprint::from_domain(&fingerprint()?)
        };
        assert!(stored.try_into_domain().is_err());
        Ok(())
    }

    #[test]
    fn unknown_fingerprint_field_is_rejected_during_deserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut json = serde_json::to_value(StoredIndexFingerprint::from_domain(&fingerprint()?))?;
        json.as_object_mut()
            .ok_or_else(|| "expected JSON object".to_string())?
            .insert("extra".to_string(), serde_json::Value::from(1));
        assert!(serde_json::from_value::<StoredIndexFingerprint>(json).is_err());
        Ok(())
    }

    #[test]
    fn lifecycle_round_trip_and_snake_case_wire_format() -> Result<(), Box<dyn std::error::Error>> {
        for lifecycle in [
            IndexLifecycle::Building,
            IndexLifecycle::Evaluated,
            IndexLifecycle::Shadow,
            IndexLifecycle::Active,
            IndexLifecycle::Retired,
            IndexLifecycle::Collectable,
            IndexLifecycle::Tombstoned,
        ] {
            let stored = StoredIndexLifecycle::from_domain(lifecycle);
            let json = serde_json::to_string(&stored)?;
            assert!(json.starts_with('"'));
            let decoded = serde_json::from_str::<StoredIndexLifecycle>(&json)?;
            assert_eq!(decoded.try_into_domain()?, lifecycle);
        }
        Ok(())
    }
}
