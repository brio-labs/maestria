use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use maestria_code_intel::{REPOSITORY_CODE_INDEX_FILENAME, RepositoryCodeIndex};
use maestria_core::{InstanceLayout, InstanceManifest};

pub(crate) fn load_repository_code_index_with_exclusions(
    layout: &InstanceLayout,
    expected_manifest: Option<&InstanceManifest>,
) -> std::result::Result<Option<Arc<RepositoryCodeIndex>>, maestria_code_intel::CodeIntelError> {
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    if !index_path.exists() {
        return Ok(None);
    }
    let index = RepositoryCodeIndex::load(&index_path)?;
    index.validate_provenance()?;
    if index.is_stale_generation(maestria_code_intel::REPOSITORY_CODE_PARSER_GENERATION) {
        return Err(maestria_code_intel::CodeIntelError::Integrity {
            context: "parser generation".to_string(),
            details: index.summary.parser_generation.clone(),
        });
    }
    if let Some(manifest) = expected_manifest {
        if index.summary.excluded_patterns != manifest.excluded_patterns {
            return Err(maestria_code_intel::CodeIntelError::Integrity {
                context: "privacy exclusions".to_string(),
                details: "repository code index exclusions differ from instance manifest"
                    .to_string(),
            });
        }
        validate_repository_sources(&index, manifest)?;
    }
    Ok(Some(Arc::new(index)))
}

fn validate_repository_sources(
    index: &RepositoryCodeIndex,
    manifest: &InstanceManifest,
) -> std::result::Result<(), maestria_code_intel::CodeIntelError> {
    let repository_root =
        canonicalize_source(Path::new(&index.summary.repository_root), "repository root")?;
    if !manifest.allows_source(&repository_root) {
        return Err(scope_error(
            "repository read scope",
            repository_root.display().to_string(),
        ));
    }

    let mut provenances = Vec::new();
    for package in &index.packages {
        provenances.push(&package.provenance);
        provenances.extend(package.dependencies.iter().map(|item| &item.provenance));
        provenances.extend(package.targets.iter().map(|item| &item.provenance));
    }
    provenances.extend(index.symbols.iter().map(|symbol| &symbol.provenance));
    for relation in &index.relations {
        provenances.push(&relation.source_provenance);
        provenances.push(&relation.target_provenance);
    }
    for provenance in provenances {
        let lexical_path = repository_root.join(&provenance.file_path);
        if !manifest.allows_source(&lexical_path) {
            return Err(scope_error(
                "repository source scope",
                provenance.file_path.clone(),
            ));
        }
        let canonical_path = canonicalize_source(&lexical_path, "repository source")?;
        if !canonical_path.starts_with(&repository_root) || !manifest.allows_source(&canonical_path)
        {
            return Err(scope_error(
                "repository source scope",
                canonical_path.display().to_string(),
            ));
        }
    }
    Ok(())
}

fn canonicalize_source(
    path: &Path,
    context: &str,
) -> std::result::Result<PathBuf, maestria_code_intel::CodeIntelError> {
    fs::canonicalize(path).map_err(|error| scope_error(context, format!("{path:?}: {error}")))
}

fn scope_error(context: &str, details: String) -> maestria_code_intel::CodeIntelError {
    maestria_code_intel::CodeIntelError::Integrity {
        context: context.to_string(),
        details,
    }
}
