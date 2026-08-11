//! Index choice operations: candidates, selection profile, and runs.

use crate::api::server::ApiContext;
use crate::api::{IndexCandidatesResponse, IndexRunResponse, IndexSelectionResponse};
use anyhow::{Result, anyhow};
use maestria_core::artifact_id_for;
use maestria_domain::{ArtifactDetected, ContentHash, DomainInput, IndexStatus, content_hash};

/// Scan a root and return its classified candidate tree.
///
/// # Cancellation
/// Cancelling drops the in-flight scan task; no state is changed.
pub(super) async fn candidates(
    context: &ApiContext,
    root: String,
) -> Result<IndexCandidatesResponse> {
    validate_scope_root(context, &root)?;
    let root_path = std::path::PathBuf::from(&root);
    let home_root = maestria_index_selection::is_home_root(&root_path);
    let tree =
        tokio::task::spawn_blocking(move || maestria_index_selection::scan_candidates(&root_path))
            .await
            .map_err(|error| anyhow!("candidate scan task failed: {error}"))??;
    Ok(IndexCandidatesResponse {
        root,
        home_root,
        tree,
    })
}

/// Load the persisted selection profile, when one exists.
///
/// # Cancellation
/// Cancelling while the blocking load is in flight leaves no state changed.
pub(super) async fn selection_get(context: &ApiContext) -> Result<IndexSelectionResponse> {
    let profile_path = context.layout.system_dir.join("index-selection.json");
    let profile =
        tokio::task::spawn_blocking(move || maestria_index_selection::load_profile(&profile_path))
            .await
            .map_err(|error| anyhow!("selection load task failed: {error}"))??;
    Ok(IndexSelectionResponse { profile })
}

/// Persist an approved selection profile.
///
/// # Cancellation
/// Cancelling while the blocking write is in flight may leave the file
/// partially written; the caller should re-save to repair it.
pub(super) async fn selection_save(
    context: &ApiContext,
    profile: maestria_index_selection::IndexSelectionProfile,
) -> Result<()> {
    validate_scope_root(context, &profile.root.display().to_string())?;
    for include in &profile.includes {
        validate_within_root(include, &profile.root)?;
    }
    for key in profile.policies.keys() {
        validate_within_root(key, &profile.root)?;
    }
    let profile_path = context.layout.system_dir.join("index-selection.json");
    tokio::task::spawn_blocking(move || {
        maestria_index_selection::save_profile(&profile_path, &profile)
    })
    .await
    .map_err(|error| anyhow!("selection save task failed: {error}"))?
}

/// Index the whitelisted files under a root through the live runtime.
///
/// Files outside every include, files already indexed with the same
/// content hash, and files refused by the per-file policy are counted as
/// skipped; the rest are submitted to the runtime.
///
/// # Cancellation
/// Cancelling stops submission between files; files already submitted are
/// processed by the runtime as usual.
pub(super) async fn run(
    context: &ApiContext,
    root: String,
    includes: Vec<String>,
    policies: std::collections::BTreeMap<String, maestria_index_selection::IndexPolicy>,
) -> Result<IndexRunResponse> {
    validate_scope_root(context, &root)?;
    let root_path = std::path::PathBuf::from(&root);
    // Client-supplied whitelist entries are untrusted boundary data: every
    // include and policy key must stay within the validated root (the same
    // containment rule the selection-save op enforces).
    for include in &includes {
        validate_within_root(std::path::Path::new(include), &root_path)?;
    }
    for key in policies.keys() {
        validate_within_root(std::path::Path::new(key), &root_path)?;
    }
    let Some(runtime) = context.runtime.clone() else {
        return Err(anyhow!("index run requires the live runtime"));
    };
    let files = tokio::task::spawn_blocking(move || {
        maestria_index_selection::collect_files(&root_path, true)
    })
    .await
    .map_err(|error| anyhow!("index collection task failed: {error}"))??;
    let include_paths: Vec<std::path::PathBuf> =
        includes.iter().map(std::path::PathBuf::from).collect();
    let policy_paths: std::collections::BTreeMap<std::path::PathBuf, _> = policies
        .into_iter()
        .map(|(path, policy)| (std::path::PathBuf::from(path), policy))
        .collect();

    let mut submitted = 0usize;
    let mut skipped = 0usize;
    for file in &files {
        let Some(include) = include_paths
            .iter()
            .filter(|include| file.starts_with(include))
            .max_by_key(|include| include.as_os_str().len())
        else {
            skipped += 1;
            continue;
        };
        let bytes = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(path = %file.display(), %error, "index run: unreadable source counted as skipped");
                skipped += 1;
                continue;
            }
        };
        let artifact_id = artifact_id_for(file, &bytes);
        let hash = match ContentHash::new(content_hash(&bytes)) {
            Ok(hash) => hash,
            Err(error) => {
                tracing::warn!(path = %file.display(), %error, "index run: invalid content hash counted as skipped");
                skipped += 1;
                continue;
            }
        };
        let state = runtime.kernel_state().await;
        let already_indexed = state.artifacts.get(&artifact_id).is_some_and(|artifact| {
            artifact.content_hash.as_ref() == Some(&hash)
                && artifact.index_status == IndexStatus::Indexed
        });
        if already_indexed {
            skipped += 1;
            continue;
        }
        let policy = if let Some(policy) = policy_paths.get(include) {
            *policy
        } else {
            maestria_index_selection::IndexPolicy::everything()
        };
        let size = bytes.len() as u64;
        if !matches!(
            maestria_index_selection::select_source(file, size, policy),
            maestria_index_selection::Selection::Index
        ) {
            skipped += 1;
            continue;
        }
        let title = if let Some(name) = file.file_name().and_then(|name| name.to_str()) {
            name.to_string()
        } else {
            "unknown".to_string()
        };
        runtime
            .submit(DomainInput::ArtifactDetected(ArtifactDetected {
                artifact_id,
                title,
                source_path: file.display().to_string(),
                source_bytes: bytes,
                content_hash: hash,
            }))
            .await
            .map_err(|error| anyhow!("submit artifact {}: {error}", file.display()))?;
        submitted += 1;
    }
    Ok(IndexRunResponse { submitted, skipped })
}

/// Reject roots outside the instance read scope.
fn validate_scope_root(context: &ApiContext, root: &str) -> Result<()> {
    let (_, manifest) = super::support::load_state_and_manifest(&context.layout)?;
    let root_path = std::path::Path::new(root);
    let canonical_root = root_path
        .canonicalize()
        .map_err(|_| anyhow!("root is outside the instance read scope: {root}"))?;
    let in_scope = manifest.read_roots.iter().any(|read_root| {
        read_root
            .canonicalize()
            .is_ok_and(|canonical_read_root| canonical_root.starts_with(canonical_read_root))
    });
    if !in_scope {
        return Err(anyhow!("root is outside the instance read scope: {root}"));
    }
    Ok(())
}

/// Reject a client-supplied path that is not contained in `root`.
///
/// Component-wise (`Path::starts_with` is not lexical): a `ParentDir`
/// component can name a path that lexically escapes the root, so it is
/// rejected before the prefix check.
fn validate_within_root(path: &std::path::Path, root: &std::path::Path) -> Result<()> {
    if path
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(anyhow!(
            "selection path {} escapes the selection root {}",
            path.display(),
            root.display()
        ));
    }
    if !path.starts_with(root) {
        return Err(anyhow!(
            "selection path {} is outside the selection root {}",
            path.display(),
            root.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "index_services_tests.rs"]
mod tests;
