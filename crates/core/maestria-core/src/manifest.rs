use crate::error::{CoreError, CoreResult};
use maestria_domain::RealmId;
use maestria_ports::RetentionPolicy;
use std::path::PathBuf;

#[path = "manifest_codec.rs"]
mod manifest_codec;
#[path = "manifest_encoding.rs"]
mod manifest_encoding;
#[path = "manifest_scope.rs"]
mod manifest_scope;

use manifest_codec::{
    ManifestFields, parse_embedding_config, parse_manifest_fields, parse_ocr_config,
    parse_visual_config, retention_policy_name,
};
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
mod tests {
    use super::*;
    use std::path::Path;
    fn test_realm_id() -> Result<RealmId, Box<dyn std::error::Error>> {
        Ok(RealmId::try_from("a".repeat(64))?)
    }

    #[test]
    fn manifest_round_trips_ordered_roots_and_exclusions() -> Result<(), Box<dyn std::error::Error>>
    {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            realm_id: test_realm_id()?,
            root: PathBuf::from("/tmp/instance"),
            read_roots: vec![PathBuf::from("/tmp/notes"), PathBuf::from("/tmp/project")],
            excluded_patterns: vec![".env".to_string(), "*.key".to_string()],
            embeddings: None,
            ocr: None,
            visual: None,
        };

        let decoded = InstanceManifest::decode(&manifest.encode())?;
        assert_eq!(decoded, manifest);
        Ok(())
    }

    #[test]
    fn embedding_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            realm_id: test_realm_id()?,
            root: PathBuf::from("/tmp/instance"),
            read_roots: vec![PathBuf::from("/tmp/instance")],
            excluded_patterns: vec![".env".to_string()],
            embeddings: Some(EmbeddingConfig {
                enabled: true,
                endpoint: "http://127.0.0.1:8080/v1/embeddings".to_string(),
                model: "local-model".to_string(),
                dimensions: 3,
                provider: "local".to_string(),
                revision: "v1".to_string(),
                artifact_hash:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                preprocessing_version: "v1".to_string(),
                remote_provider: false,
                retention_policy: RetentionPolicy::NoRetention,
            }),
            ocr: None,
            visual: None,
        };

        assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
        Ok(())
    }

    #[test]
    fn ocr_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            realm_id: test_realm_id()?,
            root: PathBuf::from("/tmp/instance"),
            read_roots: vec![PathBuf::from("/tmp/instance")],
            excluded_patterns: vec![".env".to_string()],
            embeddings: None,
            ocr: Some(OcrConfig {
                enabled: true,
                endpoint: "http://127.0.0.1:10000/v1/chat/completions".to_string(),
                model: "Unlimited-OCR".to_string(),
                provider: "baidu".to_string(),
                revision: "main".to_string(),
                artifact_hash:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                preprocessing_version: "pdf-pdftoppm-v1".to_string(),
            }),
            visual: None,
        };
        assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
        Ok(())
    }

    #[test]
    fn visual_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            realm_id: test_realm_id()?,
            root: PathBuf::from("/tmp/instance"),
            read_roots: vec![PathBuf::from("/tmp/instance")],
            excluded_patterns: vec![".env".to_string()],
            embeddings: None,
            ocr: None,
            visual: Some(VisualConfig {
                enabled: true,
                endpoint: "http://127.0.0.1:10001/v1/embeddings".to_string(),
                model: "siglip-base-patch16-224".to_string(),
                dimensions: 768,
                provider: "siglip-onnx".to_string(),
                revision: "4649052661e53c7000355844105f8a1792088239".to_string(),
                artifact_hash:
                    "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                preprocessing_version: "siglip-224-rgb-v1".to_string(),
                remote_provider: false,
                retention_policy: RetentionPolicy::NoRetention,
            }),
        };

        assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
        Ok(())
    }
    #[test]
    fn migration_requires_explicit_realm_identity_and_preserves_v1_scope()
    -> Result<(), Box<dyn std::error::Error>> {
        let v1 = "schema_version=1\nroot=/tmp/instance\nread_root=/tmp/notes\n\
            read_root=/tmp/project\nexcluded_pattern=.env\nexcluded_pattern=*.key\n";
        assert!(InstanceManifest::decode(v1).is_err());

        let migrated = InstanceManifest::migrate_v1(v1, test_realm_id()?)?;
        assert_eq!(migrated.schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(migrated.realm_id, test_realm_id()?);
        assert_eq!(
            migrated.read_roots,
            vec![PathBuf::from("/tmp/notes"), PathBuf::from("/tmp/project")]
        );
        assert_eq!(InstanceManifest::decode(&migrated.encode())?, migrated);
        Ok(())
    }

    #[test]
    fn embedding_configuration_rejects_remote_endpoint() -> Result<(), Box<dyn std::error::Error>> {
        let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nembedding_enabled=true\n\
            embedding_endpoint=https://example.com/v1/embeddings\n\
            embedding_model=remote\nembedding_dimensions=3\n";
        let result = InstanceManifest::decode(contents);
        assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
        Ok(())
    }

    #[test]
    fn embedding_configuration_rejects_partial_values() -> Result<(), Box<dyn std::error::Error>> {
        let contents = "schema_version=2\nrealm_id=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nembedding_enabled=true\n";
        let result = InstanceManifest::decode(contents);
        assert!(matches!(result, Err(CoreError::InvalidManifest { .. })));
        Ok(())
    }

    #[test]
    fn default_manifest_scopes_reads_to_instance_root() -> Result<(), Box<dyn std::error::Error>> {
        let manifest =
            InstanceManifest::default_for_root(PathBuf::from("/tmp/instance"), test_realm_id()?);
        assert_eq!(manifest.read_roots, vec![PathBuf::from("/tmp/instance")]);
        assert!(manifest.excluded_patterns.iter().any(|item| item == ".env"));
        Ok(())
    }

    #[test]
    fn source_scope_rejects_escape_and_sensitive_paths() -> Result<(), Box<dyn std::error::Error>> {
        let manifest =
            InstanceManifest::default_for_root(PathBuf::from("/tmp/instance"), test_realm_id()?);
        assert!(manifest.allows_source(Path::new("/tmp/instance/notes.md")));
        assert!(!manifest.allows_source(Path::new("/tmp/instance/../outside.md")));
        assert!(!manifest.allows_source(Path::new("/tmp/instance/.env.local")));
        assert!(!manifest.allows_source(Path::new("/tmp/other/notes.md")));
        Ok(())
    }

    #[test]
    fn source_scope_rejects_relative_escape_above_root() -> Result<(), Box<dyn std::error::Error>> {
        let manifest =
            InstanceManifest::default_for_root(PathBuf::from("workspace"), test_realm_id()?);
        assert!(manifest.allows_source(Path::new("workspace/notes.md")));
        // `..` above the root must not collapse into an in-scope path.
        assert!(!manifest.allows_source(Path::new("../workspace/notes.md")));
        assert!(!manifest.allows_source(Path::new("workspace/../outside.md")));
        assert!(!manifest.allows_source(Path::new("../secret.md")));
        Ok(())
    }
}
