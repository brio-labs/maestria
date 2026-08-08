use crate::error::{CoreError, CoreResult};
use maestria_domain::RealmId;
use maestria_ports::RetentionPolicy;
use std::path::PathBuf;

#[path = "manifest_codec.rs"]
mod manifest_codec;
#[path = "manifest_codec_sparse.rs"]
mod manifest_codec_sparse;
#[path = "manifest_encoding.rs"]
mod manifest_encoding;
#[path = "manifest_scope.rs"]
mod manifest_scope;

use manifest_codec::{
    ManifestFields, parse_embedding_config, parse_manifest_fields, parse_ocr_config,
    parse_visual_config, retention_policy_name,
};
use manifest_codec_sparse::parse_sparse_config;
use manifest_scope::{lexical_normalize, path_matches_pattern};

const MANIFEST_SCHEMA_VERSION: u32 = 2;
const DEFAULT_EXCLUSIONS: [&str; 11] = [
    ".env",
    ".env.*",
    ".ssh",
    ".gnupg",
    "secrets",
    "node_modules",
    "target",
    "dist",
    "build",
    "*.pem",
    "*.key",
];

/// Persisted, instance-scoped source access configuration.
///
/// This is a boundary DTO. It contains no filesystem behavior; callers must
/// apply its roots and exclusions through a policy implementation before
/// reading source bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub provider: String,
    pub revision: String,
    pub artifact_hash: String,
    pub preprocessing_version: String,
    pub remote_provider: bool,
    pub retention_policy: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub provider: String,
    pub revision: String,
    pub artifact_hash: String,
    pub preprocessing_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub dimensions: usize,
    pub provider: String,
    pub revision: String,
    pub artifact_hash: String,
    pub preprocessing_version: String,
    pub remote_provider: bool,
    pub retention_policy: RetentionPolicy,
}

/// Learned-sparse sidecar profile.
///
/// Unlike the embedding/visual profiles, a remote provider or retained
/// retention policy is a manifest error: sparse activation is local-only by
/// construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseProfileConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub provider: String,
    pub revision: String,
    pub artifact_hash: String,
    pub preprocessing_version: String,
    pub model: String,
    pub vocabulary_size: u32,
    pub term_cap: u32,
    pub remote_provider: bool,
    pub retention_policy: RetentionPolicy,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceManifest {
    pub schema_version: u32,
    pub realm_id: RealmId,
    pub root: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub excluded_patterns: Vec<String>,
    pub embeddings: Option<EmbeddingConfig>,
    pub ocr: Option<OcrConfig>,
    pub visual: Option<VisualConfig>,
    pub sparse: Option<SparseProfileConfig>,
}

impl InstanceManifest {
    pub fn default_for_root(root: PathBuf, realm_id: RealmId) -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            realm_id,
            read_roots: vec![root.clone()],
            root,
            excluded_patterns: DEFAULT_EXCLUSIONS
                .iter()
                .map(|item| (*item).to_string())
                .collect(),
            embeddings: None,
            ocr: None,
            visual: None,
            sparse: None,
        }
    }

    pub fn decode(contents: &str) -> CoreResult<Self> {
        let fields = parse_manifest_fields(contents)?;
        Self::from_fields(fields, MANIFEST_SCHEMA_VERSION, None)
    }

    /// Performs the explicit v1-to-v2 manifest migration.
    ///
    /// Existing manifests deliberately remain unreadable as v2 until this
    /// boundary is invoked with a newly generated, validated realm identity.
    pub fn migrate_v1(contents: &str, realm_id: RealmId) -> CoreResult<Self> {
        let fields = parse_manifest_fields(contents)?;
        Self::from_fields(fields, 1, Some(realm_id))
    }

    fn from_fields(
        fields: ManifestFields,
        expected_schema_version: u32,
        migrated_realm_id: Option<RealmId>,
    ) -> CoreResult<Self> {
        let embeddings = parse_embedding_config(&fields)?;
        let ocr = parse_ocr_config(&fields)?;
        let visual = parse_visual_config(&fields)?;
        let sparse = parse_sparse_config(&fields)?;
        let ManifestFields {
            schema_version,
            realm_id: parsed_realm_id,
            root,
            read_roots,
            excluded_patterns,
            ..
        } = fields;

        let schema_version = schema_version.ok_or_else(|| CoreError::InvalidInput {
            message: "instance manifest is missing schema_version".to_string(),
        })?;
        if schema_version != expected_schema_version {
            return Err(CoreError::InvalidInput {
                message: format!("unsupported instance manifest schema version {schema_version}"),
            });
        }
        let realm_id = match migrated_realm_id {
            Some(realm_id) if parsed_realm_id.is_none() => realm_id,
            Some(_) => {
                return Err(CoreError::InvalidManifest {
                    key: "realm_id".to_string(),
                    reason: "schema version 1 must not define a realm identity".to_string(),
                });
            }
            None => {
                let realm_id = parsed_realm_id.ok_or_else(|| CoreError::InvalidInput {
                    message: "instance manifest is missing realm_id".to_string(),
                })?;
                RealmId::try_from(realm_id).map_err(|error| CoreError::InvalidManifest {
                    key: "realm_id".to_string(),
                    reason: error.to_string(),
                })?
            }
        };
        let root = root.ok_or_else(|| CoreError::InvalidInput {
            message: "instance manifest is missing root".to_string(),
        })?;
        if read_roots.is_empty() {
            return Err(CoreError::InvalidInput {
                message: "instance manifest must define at least one read_root".to_string(),
            });
        }
        if excluded_patterns.is_empty() {
            return Err(CoreError::InvalidInput {
                message: "instance manifest must define at least one excluded_pattern".to_string(),
            });
        }

        Ok(Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            realm_id,
            root,
            read_roots,
            excluded_patterns,
            embeddings,
            ocr,
            visual,
            sparse,
        })
    }

    /// Checks source scope without touching the filesystem.
    ///
    /// Paths that lexically escape every configured root (or a root that
    /// itself escapes) are denied rather than normalized into scope.
    pub fn allows_source(&self, path: &std::path::Path) -> bool {
        let Some(normalized_path) = lexical_normalize(path) else {
            return false;
        };
        if self
            .excluded_patterns
            .iter()
            .any(|pattern| path_matches_pattern(&normalized_path, pattern))
        {
            return false;
        }
        self.read_roots.iter().any(|root| {
            lexical_normalize(root).is_some_and(|root| normalized_path.starts_with(root))
        })
    }
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
