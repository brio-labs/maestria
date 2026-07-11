use crate::error::{CoreError, CoreResult};
use std::path::PathBuf;

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
pub struct InstanceManifest {
    pub schema_version: u32,
    pub root: PathBuf,
    pub read_roots: Vec<PathBuf>,
    pub excluded_patterns: Vec<String>,
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
        lines.push(String::new());
        lines.join("\n")
    }

    pub fn decode(contents: &str) -> CoreResult<Self> {
        let mut schema_version = None;
        let mut root = None;
        let mut read_roots = Vec::new();
        let mut excluded_patterns = Vec::new();

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
                    schema_version =
                        Some(value.parse::<u32>().map_err(|_| CoreError::InvalidInput {
                            message: format!("invalid instance manifest schema version: {value}"),
                        })?);
                }
                "root" => root = Some(PathBuf::from(value)),
                "read_root" => read_roots.push(PathBuf::from(value)),
                "excluded_pattern" => excluded_patterns.push(value.to_string()),
                other => {
                    return Err(CoreError::InvalidInput {
                        message: format!("unknown instance manifest key: {other}"),
                    });
                }
            }
        }

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
    fn manifest_round_trips_ordered_roots_and_exclusions() {
        let manifest = InstanceManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            root: PathBuf::from("/tmp/instance"),
            read_roots: vec![PathBuf::from("/tmp/notes"), PathBuf::from("/tmp/project")],
            excluded_patterns: vec![".env".to_string(), "*.key".to_string()],
        };

        let decoded = InstanceManifest::decode(&manifest.encode()).expect("manifest is valid");
        assert_eq!(decoded, manifest);
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
