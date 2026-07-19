use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default)]
pub(crate) struct StoredWebEvidenceMetadata {
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    effective_at: Option<String>,
    #[serde(default)]
    accessed_at: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    primary_source: bool,
    #[serde(default)]
    is_dynamic: bool,
    #[serde(default)]
    is_paywalled: bool,
}

impl StoredWebEvidenceMetadata {
    pub(crate) fn from_domain(metadata: &maestria_domain::WebEvidenceMetadata) -> Self {
        Self {
            published_at: metadata.published_at.clone(),
            updated_at: metadata.updated_at.clone(),
            effective_at: metadata.effective_at.clone(),
            accessed_at: metadata.accessed_at.clone(),
            content_type: metadata.content_type.clone(),
            primary_source: metadata.primary_source,
            is_dynamic: metadata.is_dynamic,
            is_paywalled: metadata.is_paywalled,
        }
    }

    pub(crate) fn into_domain(self) -> maestria_domain::WebEvidenceMetadata {
        maestria_domain::WebEvidenceMetadata {
            published_at: self.published_at,
            updated_at: self.updated_at,
            effective_at: self.effective_at,
            accessed_at: self.accessed_at,
            content_type: self.content_type,
            primary_source: self.primary_source,
            is_dynamic: self.is_dynamic,
            is_paywalled: self.is_paywalled,
        }
    }
}
