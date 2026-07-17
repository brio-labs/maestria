use crate::error::{CoreError, CoreResult};
use maestria_ports::RetentionPolicy;
use std::path::PathBuf;
use url::Url;

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
    pub remote_provider: bool,
    pub retention_policy: RetentionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceManifest {
    pub schema_version: u32,
    pub root: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub excluded_patterns: Vec<String>,
    pub embeddings: Option<EmbeddingConfig>,
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
        lines.push(String::new());
        lines.join("\n")
    }
    pub fn decode(contents: &str) -> CoreResult<Self> {
        let ManifestFields {
            schema_version,
            root,
            read_roots,
            excluded_patterns,
            embedding_enabled,
            embedding_endpoint,
            embedding_model,
            embedding_dimensions,
            embedding_remote_provider,
            embedding_retention_policy,
        } = parse_manifest_fields(contents)?;

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

        let embeddings = match (
            embedding_enabled,
            embedding_endpoint,
            embedding_model,
            embedding_dimensions,
        ) {
            (None, None, None, None) => None,
            (Some(enabled), Some(endpoint), Some(model), Some(dimensions)) => {
                validate_embedding_endpoint(&endpoint)?;
                if enabled && dimensions == 0 {
                    return Err(CoreError::InvalidInput {
                        message: "embedding_dimensions must be positive when enabled".to_string(),
                    });
                }
                let remote_provider = embedding_remote_provider.is_some_and(|value| value);
                let retention_policy = parse_retention_policy(
                    embedding_retention_policy
                        .as_deref()
                        .map_or("no_retention", |value| value),
                )?;
                Some(EmbeddingConfig {
                    enabled,
                    endpoint,
                    model,
                    dimensions,
                    remote_provider,
                    retention_policy,
                })
            }
            _ => {
                return Err(CoreError::InvalidInput {
                    message: "embedding configuration must define enabled, endpoint, model, and dimensions".to_string(),
                });
            }
        };

        Ok(Self {
            schema_version,
            root,
            read_roots,
            excluded_patterns,
            embeddings,
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

struct ManifestFields {
    schema_version: Option<u32>,
    root: Option<PathBuf>,
    read_roots: Vec<PathBuf>,
    excluded_patterns: Vec<String>,
    embedding_enabled: Option<bool>,
    embedding_endpoint: Option<String>,
    embedding_model: Option<String>,
    embedding_dimensions: Option<usize>,
    embedding_remote_provider: Option<bool>,
    embedding_retention_policy: Option<String>,
}

fn parse_manifest_fields(contents: &str) -> CoreResult<ManifestFields> {
    let mut fields = ManifestFields {
        schema_version: None,
        root: None,
        read_roots: Vec::new(),
        excluded_patterns: Vec::new(),
        embedding_enabled: None,
        embedding_endpoint: None,
        embedding_model: None,
        embedding_dimensions: None,
        embedding_remote_provider: None,
        embedding_retention_policy: None,
    };
    for line in contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| CoreError::InvalidInput {
                message: format!("invalid instance manifest line: {line}"),
            })?;
        if value.is_empty() {
            return Err(CoreError::InvalidInput {
                message: format!("instance manifest value is empty for {key}"),
            });
        }
        match key {
            "schema_version" => {
                fields.schema_version =
                    Some(value.parse::<u32>().map_err(|_| CoreError::InvalidInput {
                        message: format!("invalid instance manifest schema version: {value}"),
                    })?);
            }
            "root" => fields.root = Some(PathBuf::from(value)),
            "read_root" => fields.read_roots.push(PathBuf::from(value)),
            "excluded_pattern" => fields.excluded_patterns.push(value.to_string()),
            "embedding_enabled" => {
                fields.embedding_enabled =
                    Some(value.parse::<bool>().map_err(|_| CoreError::InvalidInput {
                        message: format!("invalid embedding_enabled value: {value}"),
                    })?);
            }
            "embedding_endpoint" => fields.embedding_endpoint = Some(value.to_string()),
            "embedding_model" => fields.embedding_model = Some(value.to_string()),
            "embedding_dimensions" => {
                fields.embedding_dimensions =
                    Some(
                        value
                            .parse::<usize>()
                            .map_err(|_| CoreError::InvalidInput {
                                message: format!("invalid embedding_dimensions value: {value}"),
                            })?,
                    );
            }
            "embedding_remote_provider" => {
                fields.embedding_remote_provider =
                    Some(value.parse::<bool>().map_err(|_| CoreError::InvalidInput {
                        message: format!("invalid embedding_remote_provider value: {value}"),
                    })?);
            }
            "embedding_retention_policy" => {
                fields.embedding_retention_policy = Some(value.to_string());
            }
            other => {
                return Err(CoreError::InvalidInput {
                    message: format!("unknown instance manifest key: {other}"),
                });
            }
        }
    }
    Ok(fields)
}

fn retention_policy_name(policy: &RetentionPolicy) -> &'static str {
    match policy {
        RetentionPolicy::NoRetention => "no_retention",
        RetentionPolicy::ProviderDefined => "provider_defined",
    }
}

fn parse_retention_policy(value: &str) -> CoreResult<RetentionPolicy> {
    match value {
        "no_retention" => Ok(RetentionPolicy::NoRetention),
        "provider_defined" => Ok(RetentionPolicy::ProviderDefined),
        _ => Err(CoreError::InvalidInput {
            message: format!("invalid embedding_retention_policy value: {value}"),
        }),
    }
}
fn validate_embedding_endpoint(endpoint: &str) -> CoreResult<()> {
    let url = Url::parse(endpoint).map_err(|error| CoreError::InvalidInput {
        message: format!("invalid embedding endpoint: {error}"),
    })?;
    let valid = url.scheme() == "http"
        && matches!(url.host_str(), Some("127.0.0.1" | "::1" | "[::1]"))
        && url.path() == "/v1/embeddings"
        && url.query().is_none()
        && url.fragment().is_none();
    if !valid {
        return Err(CoreError::InvalidInput {
            message: "embedding endpoint must be an http loopback /v1/embeddings URL".to_string(),
        });
    }
    Ok(())
}

fn lexical_normalize(path: &std::path::Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_matches_pattern(path: &std::path::Path, pattern: &str) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        pattern == name
            || (pattern == ".env.*" && name.starts_with(".env."))
            || (pattern == "*.pem" && name.ends_with(".pem"))
            || (pattern == "*.key" && name.ends_with(".key"))
    })
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
                remote_provider: false,
                retention_policy: RetentionPolicy::NoRetention,
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
