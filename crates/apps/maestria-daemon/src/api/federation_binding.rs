use std::{fs, io::Write, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::RealmId;
use serde::{Deserialize, Serialize};

use super::super::protocol::FederationCredential;

/// Consumer-local connection data. It is never persisted in a domain event or
/// exposed by a list operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct FederationBinding {
    pub(super) provider_realm: RealmId,
    pub(super) provider_socket_path: PathBuf,
    pub(super) credential: FederationCredential,
}

pub(super) fn install(layout: &InstanceLayout, binding: FederationBinding) -> Result<()> {
    let directory = bindings_dir(layout);
    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "create federation binding directory {}",
            directory.display()
        )
    })?;
    super::super::set_private_directory_permissions(&directory)?;

    let target = binding_path(layout, &binding.provider_realm);
    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        binding.provider_realm.as_str(),
        std::process::id()
    ));
    let content = serde_json::to_vec(&binding).context("encode federation binding")?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&temporary) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(&temporary).with_context(|| {
                format!(
                    "remove stale federation binding temporary {}",
                    temporary.display()
                )
            })?;
            options.open(&temporary).with_context(|| {
                format!(
                    "create federation binding temporary {}",
                    temporary.display()
                )
            })?
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "create federation binding temporary {}",
                    temporary.display()
                )
            });
        }
    };
    let result = (|| -> Result<()> {
        file.write_all(&content)
            .with_context(|| format!("write federation binding {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("sync federation binding {}", temporary.display()))?;
        drop(file);
        fs::rename(&temporary, &target)
            .with_context(|| format!("install federation binding {}", target.display()))?;
        super::super::set_private_permissions(&target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn load(layout: &InstanceLayout, provider_realm: &RealmId) -> Result<FederationBinding> {
    let path = binding_path(layout, provider_realm);
    let content =
        fs::read(&path).with_context(|| format!("read federation binding {}", path.display()))?;
    let binding: FederationBinding = serde_json::from_slice(&content)
        .with_context(|| format!("decode federation binding {}", path.display()))?;
    if &binding.provider_realm != provider_realm {
        return Err(anyhow!(
            "federation binding provider realm does not match its path"
        ));
    }
    Ok(binding)
}

fn bindings_dir(layout: &InstanceLayout) -> PathBuf {
    layout.system_dir.join("federation")
}

fn binding_path(layout: &InstanceLayout, provider_realm: &RealmId) -> PathBuf {
    bindings_dir(layout).join(format!("{}.json", provider_realm.as_str()))
}
