use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use maestria_core::InstanceManifest;

use super::super::protocol::{ClientOperation, ClientResponse};
use super::super::server::{ApiContext, RequestPrincipal};

#[path = "search_roots_status.rs"]
mod search_roots_status;
pub(super) use search_roots_status::{status, status_for_roots};

const MAX_APPROVED_ROOTS: usize = 64;

pub(super) async fn dispatch(
    context: &ApiContext,
    principal: &RequestPrincipal,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    require_instance(principal)?;
    match operation {
        ClientOperation::SearchRootsStatus => {
            Ok(ClientResponse::SearchRootsStatus(status(context).await?))
        }
        ClientOperation::SearchRootAdd { root } => {
            add_root(context, &root)?;
            Ok(ClientResponse::SearchRootsStatus(status(context).await?))
        }
        ClientOperation::SearchRootRemove { root } => {
            remove_root(context, &root)?;
            Ok(ClientResponse::SearchRootsStatus(status(context).await?))
        }
        _ => Err(anyhow!("invalid search-root operation")),
    }
}

fn add_root(context: &ApiContext, requested: &str) -> Result<()> {
    let path = PathBuf::from(requested);
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("inspect approved root {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "approved root must not be a symbolic link: {}",
            path.display()
        );
    }
    if !metadata.is_dir() {
        bail!("approved root is not a directory: {}", path.display());
    }
    let root = path
        .canonicalize()
        .with_context(|| format!("canonicalize approved root {}", path.display()))?;
    let mut current = context.source_manifest.write();
    if current.read_roots.iter().any(|existing| existing == &root) {
        return Ok(());
    }
    if current
        .read_roots
        .iter()
        .any(|existing| existing.starts_with(&root) || root.starts_with(existing))
    {
        bail!(
            "approved root overlaps an existing root: {}",
            root.display()
        );
    }
    if current.read_roots.len() >= MAX_APPROVED_ROOTS {
        bail!("at most {MAX_APPROVED_ROOTS} approved roots are supported");
    }
    let mut updated = current.clone();
    updated.read_roots.push(root);
    persist_manifest(context, &updated)?;
    *current = updated;
    Ok(())
}

fn remove_root(context: &ApiContext, requested: &str) -> Result<()> {
    let path = PathBuf::from(requested);
    if !path.is_absolute() {
        bail!("approved root path must be absolute");
    }
    let root = match path.canonicalize() {
        Ok(root) => root,
        Err(_) => maestria_governance::lexical_normalize(&path)
            .ok_or_else(|| anyhow!("invalid approved root path {}", path.display()))?,
    };
    let mut current = context.source_manifest.write();
    let original_len = current.read_roots.len();
    let mut updated = current.clone();
    updated.read_roots.retain(|existing| existing != &root);
    if updated.read_roots.len() == original_len {
        bail!("approved root is not selected: {}", root.display());
    }
    persist_manifest(context, &updated)?;
    *current = updated;
    Ok(())
}

fn persist_manifest(context: &ApiContext, manifest: &InstanceManifest) -> Result<()> {
    let temporary = context.layout.manifest_path.with_extension("txt.tmp");
    fs::write(&temporary, manifest.encode())
        .with_context(|| format!("write updated manifest {}", temporary.display()))?;
    fs::rename(&temporary, &context.layout.manifest_path).with_context(|| {
        format!(
            "replace instance manifest {}",
            context.layout.manifest_path.display()
        )
    })
}

fn require_instance(principal: &RequestPrincipal) -> Result<()> {
    if matches!(principal, RequestPrincipal::Instance) {
        Ok(())
    } else {
        bail!("approved-root administration requires the instance owner token")
    }
}
