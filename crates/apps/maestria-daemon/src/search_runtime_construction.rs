use super::*;
use maestria_core::{InstanceLayout, InstanceManifest};

/// Construct the one search runtime used by CLI search and explain.
pub fn prepare_search_runtime(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_repository_policy(
        layout,
        state,
        manifest,
        retrieval_policy,
        RepositoryExecutionPolicy::Shadow,
    )
}

/// Construct a search runtime with a verified repository benchmark policy.
pub fn prepare_search_runtime_with_repository_policy(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    repository_execution_policy: RepositoryExecutionPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_options(
        layout,
        state,
        manifest,
        retrieval_policy,
        repository_execution_policy,
        true,
        false,
    )
}

/// Construct a search runtime without rebuilding writable projections.
pub fn prepare_search_runtime_read_only(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_read_only_with_repository_policy(
        layout,
        state,
        manifest,
        retrieval_policy,
        RepositoryExecutionPolicy::Shadow,
    )
}

/// Construct the federation search runtime with read-only stores and no
/// graph, dense projection, or persistent shadow-observation adapters.
pub fn prepare_search_runtime_read_only_for_federation(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_options(
        layout,
        state,
        manifest,
        retrieval_policy,
        RepositoryExecutionPolicy::Shadow,
        false,
        true,
    )
}

/// Construct a read-only search runtime with a verified repository policy.
pub fn prepare_search_runtime_read_only_with_repository_policy(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    repository_execution_policy: RepositoryExecutionPolicy,
) -> Result<Arc<SearchRuntime>> {
    prepare_search_runtime_with_options(
        layout,
        state,
        manifest,
        retrieval_policy,
        repository_execution_policy,
        false,
        false,
    )
}

fn prepare_search_runtime_with_options(
    layout: &InstanceLayout,
    state: &KernelState,
    manifest: &InstanceManifest,
    retrieval_policy: maestria_governance::RetrievalSecurityPolicy,
    repository_execution_policy: RepositoryExecutionPolicy,
    allow_projection_writes: bool,
    federation_read_only: bool,
) -> Result<Arc<SearchRuntime>> {
    SearchRuntime::assemble(
        layout,
        state,
        manifest,
        retrieval_policy,
        repository_execution_policy,
        allow_projection_writes,
        federation_read_only,
    )
}

use maestria_code_intel::{REPOSITORY_CODE_INDEX_FILENAME, RepositoryCodeIndex};
use std::path::Path;

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
            details: index.summary.parser_generation.as_str().to_string(),
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
) -> std::result::Result<std::path::PathBuf, maestria_code_intel::CodeIntelError> {
    std::fs::canonicalize(path).map_err(|error| scope_error(context, format!("{path:?}: {error}")))
}

fn scope_error(context: &str, details: String) -> maestria_code_intel::CodeIntelError {
    maestria_code_intel::CodeIntelError::Integrity {
        context: context.to_string(),
        details,
    }
}
