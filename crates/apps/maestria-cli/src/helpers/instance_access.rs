use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use maestria_core::InstanceManifest;
use std::fs;
use std::path::PathBuf;

pub(crate) fn ensure_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    maestria_daemon::prepare_instance(instance_dir)
}

pub(crate) fn validated_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let layout = InstanceLayout::for_root(instance_dir);
    if !layout.manifest_path.exists() {
        return Err(anyhow::anyhow!(
            "instance manifest is missing at {}; run init first",
            layout.manifest_path.display()
        ));
    }
    load_manifest(&layout)?;
    Ok(layout)
}

pub(crate) fn load_manifest(layout: &InstanceLayout) -> Result<InstanceManifest> {
    let contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    maestria_core::InstanceService::parse_manifest(&contents)
        .map_err(|error| anyhow::anyhow!("parse instance manifest: {error}"))
}
