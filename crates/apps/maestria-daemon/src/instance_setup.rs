use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceManifest, InstanceService};
use maestria_domain::{DomainInput, KernelState, replay_events};
use maestria_governance::PrivacyExclusions;
use maestria_ports::EventFilter;
use maestria_storage_sqlite::SqliteStore;

use crate::recovery_inputs::RecoveryInputs;

/// Validate that every pending `ResumeParser` source path is within the
/// instance manifest read scope before the daemon touches blobs or runtime
/// infrastructure. Out-of-scope and excluded pending parsers fail fast
/// with a descriptive error, avoiding useless blob reads and runtime work.
pub fn validate_recovery_scope(layout: &InstanceLayout, recovery: &RecoveryInputs) -> Result<()> {
    let manifest_contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    let manifest = InstanceManifest::decode(&manifest_contents)
        .map_err(|error| anyhow!("parse instance manifest for recovery scope: {error}"))?;
    let privacy = PrivacyExclusions::default();

    for input in &recovery.resume_parsers {
        if let DomainInput::ResumeParser(record) = input {
            let source = std::path::Path::new(&record.source_path);
            if !manifest.allows_source(source) {
                return Err(anyhow!(
                    "resume parser source path is outside the instance manifest read scope \
                     or excluded by pattern: {} (artifact {} \"{}\")",
                    record.source_path,
                    record.artifact_id,
                    record.title,
                ));
            }
            if privacy.is_excluded(source) {
                return Err(anyhow!(
                    "resume parser source path is excluded by privacy policy: \
                     {} (artifact {} \"{}\")",
                    record.source_path,
                    record.artifact_id,
                    record.title,
                ));
            }
        }
    }
    Ok(())
}

pub fn prepare_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let plan = InstanceService::init_instance(InitInstanceInput { root: instance_dir })?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    if !plan.manifest_path.exists() {
        fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    }
    Ok(plan.layout)
}

pub fn load_kernel_state(layout: &InstanceLayout) -> Result<KernelState> {
    let sqlite_store = if layout.database_path.exists() {
        SqliteStore::open_read_only(&layout.database_path)
    } else {
        SqliteStore::open(&layout.database_path)
    }
    .with_context(|| {
        format!(
            "open sqlite store for replay {}",
            layout.database_path.display()
        )
    })?;
    let events =
        maestria_ports::EventLog::scan(&sqlite_store, EventFilter { artifact_id: None })
            .with_context(|| format!("scan domain events {}", layout.database_path.display()))?;
    replay_events(&events).map_err(|error| anyhow!(error))
}

#[cfg(test)]
#[path = "recovery_scope_tests.rs"]
mod tests;
