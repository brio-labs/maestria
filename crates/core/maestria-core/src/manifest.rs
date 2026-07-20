use crate::error::{CoreError, CoreResult};
use maestria_ports::RetentionPolicy;
use std::path::PathBuf;

#[path = "manifest_codec.rs"]
mod manifest_codec;
#[path = "manifest_scope.rs"]
mod manifest_scope;

use manifest_codec::{
    ManifestFields, parse_embedding_config, parse_manifest_fields, parse_ocr_config,
    retention_policy_name,
};
use manifest_scope::{lexical_normalize, path_matches_pattern};

const MANIFEST_SCHEMA_VERSION: u32 = 1;
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
pub struct InstanceManifest {
    pub schema_version: u32,
    pub root: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub excluded_patterns: Vec<String>,
    pub embeddings: Option<EmbeddingConfig>,
    pub ocr: Option<OcrConfig>,
}

impl InstanceManifest {
    pub fn default_for_root(root: PathBuf) -> Self {
        Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            read_roots: vec![root.clone()],
            root,
            excluded_patterns: DEFAULT_EXCLUSIONS
                .iter()
                .map(|item| (*item).to_string())
                .collect(),
            embeddings: None,
            ocr: None,
        }
    }

    pub fn encode(&self) -> String {
        let mut lines = vec![
            format!("schema_version={}", self.schema_version),
            format!("root={}", self.root.display()),
        ];
        lines.extend(
            self.read_roots
                .iter()
                .map(|root| format!("read_root={}", root.display())),
        );
        lines.extend(
            self.excluded_patterns
                .iter()
                .map(|pattern| format!("excluded_pattern={pattern}")),
        );
        if let Some(embeddings) = &self.embeddings {
            lines.push(format!("embedding_enabled={}", embeddings.enabled));
            lines.push(format!("embedding_endpoint={}", embeddings.endpoint));
            lines.push(format!("embedding_provider={}", embeddings.provider));
            lines.push(format!("embedding_revision={}", embeddings.revision));
            lines.push(format!(
                "embedding_artifact_hash={}",
                embeddings.artifact_hash
            ));
            lines.push(format!(
                "embedding_preprocessing_version={}",
                embeddings.preprocessing_version
            ));
            lines.push(format!(
                "embedding_remote_provider={}",
                embeddings.remote_provider
            ));
            lines.push(format!(
                "embedding_retention_policy={}",
                retention_policy_name(&embeddings.retention_policy)
            ));
            lines.push(format!("embedding_model={}", embeddings.model));
            lines.push(format!("embedding_dimensions={}", embeddings.dimensions));
        }
        if let Some(ocr) = &self.ocr {
            lines.push(format!("ocr_enabled={}", ocr.enabled));
            lines.push(format!("ocr_endpoint={}", ocr.endpoint));
            lines.push(format!("ocr_provider={}", ocr.provider));
            lines.push(format!("ocr_revision={}", ocr.revision));
            lines.push(format!("ocr_artifact_hash={}", ocr.artifact_hash));
            lines.push(format!(
                "ocr_preprocessing_version={}",
                ocr.preprocessing_version
            ));
            lines.push(format!("ocr_model={}", ocr.model));
        }
        lines.push(String::new());
        lines.join("\n")
    }

    pub fn decode(contents: &str) -> CoreResult<Self> {
        let fields = parse_manifest_fields(contents)?;
        let embeddings = parse_embedding_config(&fields)?;
        let ocr = parse_ocr_config(&fields)?;
        let ManifestFields {
            schema_version,
            root,
            read_roots,
            excluded_patterns,
            ..
        } = fields;

        let schema_version = schema_version.ok_or_else(|| CoreError::InvalidInput {
            message: "instance manifest is missing schema_version".to_string(),
        })?;
        if schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(CoreError::InvalidInput {
                message: format!("unsupported instance manifest schema version {schema_version}"),
            });
        }
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
            schema_version,
            root,
            read_roots,
            excluded_patterns,
            embeddings,
            ocr,
        })
    }

    /// Checks source scope without touching the filesystem.
    pub fn allows_source(&self, path: &std::path::Path) -> bool {
        let normalized_path = lexical_normalize(path);
        if self
            .excluded_patterns
            .iter()
            .any(|pattern| path_matches_pattern(&normalized_path, pattern))
        {
            return false;
        }
        self.read_roots
            .iter()
            .map(|root| lexical_normalize(root))
            .any(|root| normalized_path.starts_with(root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn manifest_round_trips_ordered_roots_and_exclusions() -> Result<(), Box<dyn std::error::Error>>
    {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            root: PathBuf::from("/tmp/instance"),
            read_roots: vec![PathBuf::from("/tmp/notes"), PathBuf::from("/tmp/project")],
            excluded_patterns: vec![".env".to_string(), "*.key".to_string()],
            embeddings: None,
            ocr: None,
        };

        let decoded = InstanceManifest::decode(&manifest.encode())?;
        assert_eq!(decoded, manifest);
        Ok(())
    }

    #[test]
    fn embedding_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
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
        };

        assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
        Ok(())
    }

    #[test]
    fn ocr_configuration_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
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
        };

        assert_eq!(InstanceManifest::decode(&manifest.encode())?, manifest);
        Ok(())
    }

    #[test]
    fn embedding_configuration_rejects_remote_endpoint() {
        let contents = "schema_version=1\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nembedding_enabled=true\n\
            embedding_endpoint=https://example.com/v1/embeddings\n\
            embedding_model=remote\nembedding_dimensions=3\n";
        let result = InstanceManifest::decode(contents);
        assert!(matches!(result, Err(CoreError::InvalidInput { .. })));
    }

    #[test]
    fn embedding_configuration_rejects_partial_values() {
        let contents = "schema_version=1\nroot=/tmp/instance\nread_root=/tmp/instance\n\
            excluded_pattern=.env\nembedding_enabled=true\n";
        let result = InstanceManifest::decode(contents);
        assert!(matches!(result, Err(CoreError::InvalidInput { .. })));
    }

    #[test]
    fn default_manifest_scopes_reads_to_instance_root() {
        let manifest = InstanceManifest::default_for_root(PathBuf::from("/tmp/instance"));
        assert_eq!(manifest.read_roots, vec![PathBuf::from("/tmp/instance")]);
        assert!(manifest.excluded_patterns.iter().any(|item| item == ".env"));
    }

    #[test]
    fn source_scope_rejects_escape_and_sensitive_paths() {
        let manifest = InstanceManifest::default_for_root(PathBuf::from("/tmp/instance"));
        assert!(manifest.allows_source(Path::new("/tmp/instance/notes.md")));
        assert!(!manifest.allows_source(Path::new("/tmp/instance/../outside.md")));
        assert!(!manifest.allows_source(Path::new("/tmp/instance/.env.local")));
        assert!(!manifest.allows_source(Path::new("/tmp/other/notes.md")));
    }
}
