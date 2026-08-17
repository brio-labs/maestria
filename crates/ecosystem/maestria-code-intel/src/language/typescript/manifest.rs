//! Bounded `package.json` parsing: name/version plus entry points
//! (`main`/`module`/`exports`). Never executes anything and never installs;
//! bounded to the fields discovery consumes. The `exports` field is
//! arbitrary JSON, so string leaves are collected through a typed untagged
//! enum; non-string/array/object shapes are skipped (documented
//! degradation) rather than treated as parse failures.

use crate::CodeIntelError;
use crate::identity::RepositoryIdentity;
use crate::types::{RecordProvenance, SourceRange};
use maestria_domain::content_hash;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Typed view of the `exports` field: string paths, arrays, and condition
/// objects recurse. `null` and absent exports are `None`; other shapes
/// (numbers, booleans) make the manifest unparseable and degrade like any
/// other malformed `package.json` (documented).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ExportsField {
    Path(String),
    Paths(Vec<ExportsField>),
    Conditions(BTreeMap<String, ExportsField>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct PackageJson {
    pub(crate) name: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) main: Option<String>,
    pub(crate) module: Option<String>,
    pub(crate) exports: Option<ExportsField>,
}

/// Package identity and entry points extracted from one manifest.
pub(crate) struct WebPackageIdentity {
    pub(crate) name: String,
    pub(crate) version: String,
    /// Entry-point paths from `main`, `module`, and every string leaf of
    /// `exports`, in field order, deduplicated.
    pub(crate) entry_points: Vec<String>,
}

/// Bounded name/version/entry-point read of one `package.json`. A parse
/// failure is a typed error the caller degrades per manifest position (root
/// hard, nested warned).
pub(crate) fn read_package_identity(manifest: &Path) -> Result<WebPackageIdentity, CodeIntelError> {
    let source = fs::read_to_string(manifest).map_err(|error| CodeIntelError::Io {
        operation: "read web package manifest".to_string(),
        path: manifest.to_string_lossy().into_owned(),
        details: error.to_string(),
    })?;
    let parsed: PackageJson =
        serde_json::from_str(&source).map_err(|error| CodeIntelError::Parse {
            context: format!("package.json {}", manifest.display()),
            details: error.to_string(),
        })?;
    let fallback = match manifest
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
    {
        Some(name) => name.to_string(),
        None => "web-repository".to_string(),
    };
    let mut entry_points = Vec::new();
    if let Some(main) = parsed.main.as_deref() {
        push_entry(&mut entry_points, main);
    }
    if let Some(module) = parsed.module.as_deref() {
        push_entry(&mut entry_points, module);
    }
    if let Some(exports) = parsed.exports.as_ref() {
        collect_export_paths(exports, &mut entry_points);
    }
    Ok(WebPackageIdentity {
        name: match parsed.name {
            Some(name) => name,
            None => fallback,
        },
        version: match parsed.version {
            Some(version) => version,
            None => "0.0.0".to_string(),
        },
        entry_points,
    })
}

fn push_entry(entry_points: &mut Vec<String>, path: &str) {
    let path = path.trim();
    if !path.is_empty() && !entry_points.iter().any(|existing| existing == path) {
        entry_points.push(path.to_string());
    }
}

fn collect_export_paths(field: &ExportsField, out: &mut Vec<String>) {
    match field {
        ExportsField::Path(path) => push_entry(out, path),
        ExportsField::Paths(paths) => {
            for path in paths {
                collect_export_paths(path, out);
            }
        }
        ExportsField::Conditions(conditions) => {
            for path in conditions.values() {
                collect_export_paths(path, out);
            }
        }
    }
}

pub(crate) fn manifest_provenance(
    manifest: &Path,
    identity: &RepositoryIdentity,
    parser_generation: &str,
) -> Result<RecordProvenance, CodeIntelError> {
    let bytes = fs::read(manifest).map_err(|error| CodeIntelError::Io {
        operation: "read web manifest for provenance".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write_manifest(
        source: &str,
    ) -> Result<(tempfile::TempDir, std::path::PathBuf), Box<dyn std::error::Error>> {
        let dir = tempdir()?;
        let path = dir.path().join("package.json");
        fs::write(&path, source)?;
        Ok((dir, path))
    }

    #[test]
    fn identity_reads_name_version_and_entry_points() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, manifest) = write_manifest(
            r#"{
  "name": "demo",
  "version": "1.2.3",
  "main": "dist/index.js",
  "module": "dist/index.mjs",
  "exports": {
    ".": "./dist/index.js",
    "./button": { "import": "./dist/button.js", "require": "./dist/button.cjs" }
  }
}"#,
        )?;
        let identity = read_package_identity(&manifest)?;
        assert_eq!(identity.name, "demo");
        assert_eq!(identity.version, "1.2.3");
        assert_eq!(
            identity.entry_points,
            vec![
                "dist/index.js".to_string(),
                "dist/index.mjs".to_string(),
                "./dist/index.js".to_string(),
                "./dist/button.js".to_string(),
                "./dist/button.cjs".to_string()
            ]
        );
        Ok(())
    }

    #[test]
    fn identity_falls_back_without_name_and_version() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, manifest) = write_manifest("{\"private\": true}")?;
        let identity = read_package_identity(&manifest)?;
        let parent_name = manifest
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .map_or("", |name| name);
        assert_eq!(identity.name, parent_name);
        assert_eq!(identity.version, "0.0.0");
        assert!(identity.entry_points.is_empty());
        Ok(())
    }

    #[test]
    fn null_exports_is_absent_not_fatal() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, manifest) =
            write_manifest(r#"{"name": "x", "exports": null, "main": "index.js"}"#)?;
        let identity = read_package_identity(&manifest)?;
        assert_eq!(identity.entry_points, vec!["index.js".to_string()]);
        Ok(())
    }

    #[test]
    fn non_path_exports_shape_is_a_typed_error() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, manifest) = write_manifest(r#"{"name": "x", "exports": 42}"#)?;
        assert!(read_package_identity(&manifest).is_err());
        Ok(())
    }

    #[test]
    fn broken_manifest_is_a_typed_error() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, manifest) = write_manifest("{ not json")?;
        assert!(read_package_identity(&manifest).is_err());
        Ok(())
    }
}
