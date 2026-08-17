//! Bounded Python manifest parsing: `pyproject.toml` (PEP 621
//! `[project]`), `setup.cfg` (`[metadata]`), and `setup.py` keyword regexes.
//! Never executes anything; bounded to the fields discovery consumes.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::types::{RecordProvenance, SourceRange};
use maestria_domain::content_hash;
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub(crate) struct PyProject {
    pub(crate) project: Option<PyProjectMetadata>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PyProjectMetadata {
    pub(crate) name: Option<String>,
    pub(crate) version: Option<String>,
    #[serde(default)]
    pub(crate) dependencies: Vec<String>,
}

/// Distribution name/version extracted from a bounded manifest read.
pub(crate) struct DistributionIdentity {
    pub(crate) name: String,
    pub(crate) version: String,
}

/// Bounded name/version read of one manifest. `pyproject.toml` parses as
/// TOML; `setup.cfg` reads `[metadata]`; `setup.py` regexes `name=`/`version=`
/// without executing.
pub(crate) fn read_distribution_identity(
    manifest: &Path,
) -> Result<DistributionIdentity, CodeIntelError> {
    let file_name = manifest
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("", |v| v);
    let fallback = match manifest
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
    {
        Some(name) => name.to_string(),
        None => "python-repository".to_string(),
    };
    match file_name {
        "pyproject.toml" => {
            let source = fs::read_to_string(manifest).map_err(|error| CodeIntelError::Io {
                operation: "read python manifest".to_string(),
                path: manifest.to_string_lossy().into_owned(),
                details: error.to_string(),
            })?;
            let parsed: PyProject =
                toml::from_str(&source).map_err(|error| CodeIntelError::Parse {
                    context: format!("pyproject.toml {}", manifest.display()),
                    details: error.to_string(),
                })?;
            let metadata = match parsed.project {
                Some(project) => project,
                None => PyProjectMetadata {
                    name: None,
                    version: None,
                    dependencies: Vec::new(),
                },
            };
            Ok(DistributionIdentity {
                name: match metadata.name {
                    Some(name) => name,
                    None => fallback,
                },
                version: match metadata.version {
                    Some(version) => version,
                    None => "0.0.0".to_string(),
                },
            })
        }
        "setup.cfg" => {
            let source = fs::read_to_string(manifest).map_err(|error| CodeIntelError::Io {
                operation: "read python manifest".to_string(),
                path: manifest.to_string_lossy().into_owned(),
                details: error.to_string(),
            })?;
            let (name, version) = read_setup_cfg_metadata(&source);
            Ok(DistributionIdentity {
                name: match name {
                    Some(name) => name,
                    None => fallback,
                },
                version: match version {
                    Some(version) => version,
                    None => "0.0.0".to_string(),
                },
            })
        }
        "setup.py" => {
            let source = fs::read_to_string(manifest).map_err(|error| CodeIntelError::Io {
                operation: "read python manifest".to_string(),
                path: manifest.to_string_lossy().into_owned(),
                details: error.to_string(),
            })?;
            let name = setup_py_keyword(&source, "name");
            let version = setup_py_keyword(&source, "version");
            Ok(DistributionIdentity {
                name: match name {
                    Some(name) => name,
                    None => fallback,
                },
                version: match version {
                    Some(version) => version,
                    None => "0.0.0".to_string(),
                },
            })
        }
        _ => Err(CodeIntelError::Parse {
            context: "python manifest".to_string(),
            details: format!("unexpected manifest name {file_name}"),
        }),
    }
}

/// `[metadata]` name/version from a configparser-style `setup.cfg`, bounded
/// to the two keys.
fn read_setup_cfg_metadata(source: &str) -> (Option<String>, Option<String>) {
    let mut section = String::new();
    let mut name = None;
    let mut version = None;
    for line in source.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']).to_string();
            continue;
        }
        if section != "metadata" {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().trim_matches(['"', '\'']);
            match key.trim() {
                "name" => name = Some(value.to_string()),
                "version" => version = Some(value.to_string()),
                _ => {}
            }
        }
    }
    (name, version)
}

/// `key = "value"` keyword argument from `setup.py`, regexed without
/// executing the file. Bounded and documented: dynamic values (variables,
/// function calls) fall back to the distribution default.
fn setup_py_keyword(source: &str, key: &str) -> Option<String> {
    let pattern = format!(r#"{key}\s*=\s*["']([^"']+)["']"#);
    let regex = regex::Regex::new(&pattern).ok()?;
    regex
        .captures(source)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().to_string())
}

/// `[project].dependencies` entries as (name, version spec) pairs. Entries
/// that are not a plain `name` or `name<spec>` (URLs) are skipped.
pub(crate) fn read_dependencies(
    manifest: &Path,
) -> Result<Vec<DependencyIdentity>, CodeIntelError> {
    let file_name = manifest
        .file_name()
        .and_then(|name| name.to_str())
        .map_or("", |v| v);
    if file_name != "pyproject.toml" {
        return Ok(Vec::new());
    }
    let source = fs::read_to_string(manifest).map_err(|error| CodeIntelError::Io {
        operation: "read python manifest".to_string(),
        path: manifest.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let parsed: PyProject = toml::from_str(&source).map_err(|error| CodeIntelError::Parse {
        context: format!("pyproject.toml {}", manifest.display()),
        details: error.to_string(),
    })?;
    let mut dependencies = Vec::new();
    if let Some(project) = parsed.project {
        for entry in project.dependencies {
            if let Some(dependency) = parse_dependency_entry(&entry) {
                dependencies.push(dependency);
            }
        }
    }
    Ok(dependencies)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DependencyIdentity {
    pub(crate) name: String,
    pub(crate) version_req: String,
}

fn parse_dependency_entry(entry: &str) -> Option<DependencyIdentity> {
    let entry = entry.trim();
    if entry.is_empty() || entry.starts_with("http://") || entry.starts_with("https://") {
        return None;
    }
    let name: String = entry
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
        .collect();
    if name.is_empty() {
        return None;
    }
    let version_req = entry[name.len()..].trim().to_string();
    Some(DependencyIdentity { name, version_req })
}

pub(crate) fn manifest_provenance(
    manifest: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
) -> Result<RecordProvenance, CodeIntelError> {
    let bytes = fs::read(manifest).map_err(|error| CodeIntelError::Io {
        operation: "read python manifest for provenance".to_string(),
        path: manifest.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let file_path = match manifest.strip_prefix(Path::new(&identity.root)) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => manifest.to_string_lossy().into_owned(),
    };
    Ok(RecordProvenance {
        repository_root: identity.root.clone(),
        commit_sha: identity.commit.clone(),
        worktree_identity: identity.worktree_identity.clone(),
        content_hash: content_hash(&bytes),
        file_path,
        source_range: SourceRange::new(1, 1)?,
        parser_generation: crate::types::ParserGeneration::new(parser_generation),
    })
}
