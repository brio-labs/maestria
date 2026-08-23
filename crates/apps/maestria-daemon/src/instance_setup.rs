use std::{fs, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use maestria_core::{InitInstanceInput, InstanceLayout, InstanceManifest, InstanceService};
use maestria_domain::{DomainInput, KernelState, RealmId, replay_events};
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

/// Generates the stable realm identity at the application boundary. The
/// deterministic domain only accepts the already-validated value.
pub fn generate_realm_id() -> Result<RealmId> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| anyhow!("generate realm identity: {error}"))?;
    let encoded = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    RealmId::try_from(encoded)
        .map_err(|error| anyhow!("validate generated realm identity: {error}"))
}

pub fn prepare_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let plan = InstanceService::init_instance(InitInstanceInput {
        root: instance_dir,
        realm_id: generate_realm_id()?,
    })?;
    for directory in &plan.directories {
        fs::create_dir_all(directory)?;
    }
    if !plan.manifest_path.exists() {
        fs::write(&plan.manifest_path, plan.manifest_contents.as_bytes())?;
    }
    Ok(plan.layout)
}

/// Prepare an instance layout with explicit read roots (idempotent: an
/// existing manifest is kept, matching [`prepare_instance`]).
pub fn prepare_instance_with_roots(
    instance_dir: PathBuf,
    read_roots: Vec<PathBuf>,
) -> Result<InstanceLayout> {
    let plan =
        InstanceService::init_instance_with_roots(instance_dir, read_roots, generate_realm_id()?)?;
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
    replay_events(events).map_err(|error| anyhow!(error))
}

/// Load the kernel-state slice consumed by read-only search assembly.
///
/// The read-only retrieval runtime reads only `index_generations` from
/// kernel state — lexical/dense/sparse lane eligibility and generation
/// identities; candidate authorization and evidence rendering hit the
/// stores directly. Rebuilding that slice needs just the self-contained
/// generation events, skipping the full event-log replay (O(event log)
/// with chunk-text payloads) on every search invocation.
///
/// The returned state MUST NOT be used where other slices are read: task
/// validation, output rendering, and any runtime execution need
/// [`load_kernel_state`] instead.
pub fn load_search_generations_state(layout: &InstanceLayout) -> Result<KernelState> {
    let sqlite_store = SqliteStore::open_read_only(&layout.database_path).with_context(|| {
        format!(
            "open sqlite store for search generations {}",
            layout.database_path.display()
        )
    })?;
    let events = sqlite_store.scan_index_generation_events()?;
    let index_generations =
        maestria_domain::replay_index_generations(&events).map_err(|error| anyhow!(error))?;
    let mut state = KernelState::new();
    state.index_generations = index_generations;
    Ok(state)
}

#[cfg(test)]
#[path = "recovery_scope_tests.rs"]
mod tests;
