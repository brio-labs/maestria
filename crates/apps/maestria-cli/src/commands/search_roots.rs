use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use maestria_core::InstanceLayout;
use maestria_daemon::{ClientOperation, ClientResponse, DaemonClient};

use crate::cli_types::SearchRootCommands;

/// Inspect or explicitly change the daemon-owned live-search roots.
///
/// # Cancellation
/// Dropping this future closes the owner-authenticated daemon request; a root
/// update already accepted by the daemon remains durable and takes effect.
pub async fn run(instance_dir: PathBuf, command: SearchRootCommands) -> Result<()> {
    let layout = InstanceLayout::for_root(instance_dir);
    let client = DaemonClient::from_instance(&layout).context("load daemon owner credential")?;
    let operation = match command {
        SearchRootCommands::Status => ClientOperation::SearchRootsStatus,
        SearchRootCommands::Add { root } => ClientOperation::SearchRootAdd {
            root: approved_root_argument(&root)?.display().to_string(),
        },
        SearchRootCommands::Remove { root } => ClientOperation::SearchRootRemove {
            root: revoked_root_argument(&root)?.display().to_string(),
        },
    };
    let response = client.request(operation).await?;
    let ClientResponse::SearchRootsStatus(status) = response else {
        bail!("daemon returned a non-root-status response");
    };
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

fn approved_root_argument(path: &Path) -> Result<PathBuf> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("inspect root {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "approved root must not be a symbolic link: {}",
            path.display()
        );
    }
    if !metadata.is_dir() {
        bail!("approved root is not a directory: {}", path.display());
    }
    path.canonicalize()
        .with_context(|| format!("canonicalize root {}", path.display()))
}

fn revoked_root_argument(path: &Path) -> Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let absolute =
        std::path::absolute(path).with_context(|| format!("resolve root {}", path.display()))?;
    maestria_governance::lexical_normalize(&absolute)
        .ok_or_else(|| anyhow::anyhow!("invalid root path {}", path.display()))
}
