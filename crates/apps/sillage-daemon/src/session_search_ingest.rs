use std::{path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow, bail};
use sillage_core::InstanceManifest;
use sillage_domain::{SourceChoice, content_hash};
use sillage_search_tantivy::RamMemoryBudget;
use tokio_util::sync::CancellationToken;

use super::{
    SessionCoverage,
    deadline::SessionDeadline,
    retained::{RetainedDocumentaryStore, RetainedSources},
    source_admission::{SourceLease, validate_source_path},
};
use crate::SearchRuntime;

#[path = "session_search_ingest_adapters.rs"]
mod adapters;
#[path = "session_search_ingest_builder.rs"]
mod builder;
#[cfg(test)]
pub(in crate::session_search) use builder::{
    PARSE_TEST_SERIAL, ParseCheckpointGate, clear_gate, install_gate,
};
#[path = "session_search_ingest_scan.rs"]
mod scan;
#[path = "session_search_ingest_store.rs"]
mod store;
#[path = "session_search_ingest_verification.rs"]
mod verification;

pub(super) use adapters::{EvidenceOpenContext, SessionStores};
use builder::SessionBuilder;
pub(super) use scan::{RetainedSourceValidation, scope_contains, verify_retained_source};
pub(super) use verification::VerificationBudgetExceeded;

pub(super) struct PreparedSession {
    pub(super) runtime: SearchRuntime,
    pub(super) stores: Arc<SessionStores>,
    pub(super) runtime_admission: Arc<Vec<sillage_search_tantivy::RamMemoryReservation>>,
    pub(super) retained_sources: Arc<RetainedSources>,
    pub(super) choice: SourceChoice,
    pub(super) selected_path: PathBuf,
    pub(super) source_root: PathBuf,
    pub(super) manifest: InstanceManifest,
    pub(super) coverage: SessionCoverage,
    pub(super) source_lease: SourceLease,
    pub(super) retained_generation: Option<super::retained::PersistedGeneration>,
}

pub(super) fn prepare(
    choice: SourceChoice,
    cancellation: CancellationToken,
    memory: Arc<RamMemoryBudget>,
    documentary_root: Option<Arc<PathBuf>>,
    deadline: SessionDeadline,
    source_lease: SourceLease,
) -> Result<PreparedSession> {
    super::validate_session_choice(&choice, documentary_root.is_some())?;
    ensure_not_cancelled(&cancellation)?;
    let (selected, root) = scan::validate_selected_path(choice.scope())?;
    validate_source_path(selected)?;
    let source_bytes = selected
        .as_os_str()
        .len()
        .checked_add(root.as_os_str().len())
        .ok_or_else(|| anyhow!("session source path size overflow"))?;
    let setup_bytes = u64::try_from(source_bytes)?
        .checked_mul(8)
        .and_then(|bytes| bytes.checked_add(49_152))
        .ok_or_else(|| anyhow!("session source setup admission overflow"))?;
    let _setup = memory.reserve(setup_bytes)?;
    if sillage_index_selection::is_privacy_excluded_path(selected) {
        bail!("selected source is excluded by the local privacy policy");
    }
    let selected_path = selected.to_path_buf();
    let source_root = root.to_path_buf();
    let root_text = root
        .to_str()
        .ok_or_else(|| anyhow!("selected source root is not UTF-8"))?;
    let realm_hash = content_hash(root_text.as_bytes());
    let realm_id = sillage_domain::RealmId::try_from(
        realm_hash
            .strip_prefix("sha256:")
            .ok_or_else(|| anyhow!("source scope identity could not be created"))?
            .to_owned(),
    )?;
    let manifest = InstanceManifest::default_for_root(source_root.clone(), realm_id);
    let stores = Arc::new(SessionStores::new(memory.clone())?);
    let documentary_store = documentary_root
        .map(|root| RetainedDocumentaryStore::new(root.as_ref().clone(), &selected_path))
        .transpose()?;
    let mut builder = SessionBuilder::new(
        &choice,
        stores.clone(),
        selected_path.clone(),
        documentary_store.as_ref(),
        deadline,
    );
    ensure_not_cancelled(&cancellation)?;
    scan::scan_selected_source(&choice, &selected_path, &cancellation, &mut builder)?;
    let runtime = SearchRuntime::from_session_parts(stores.runtime_parts(), manifest.clone())?
        .with_allowed_roots(Some(std::slice::from_ref(&source_root)));
    let runtime_admission = Arc::new(std::mem::take(&mut builder.runtime_admission));
    let retained_sources = Arc::new(std::mem::replace(
        &mut builder.retained_sources,
        RetainedSources::new(),
    ));
    let retained_generation = builder.retained_generation.take();
    let coverage = std::mem::replace(&mut builder.coverage, SessionCoverage::new());
    drop(builder);
    Ok(PreparedSession {
        runtime,
        runtime_admission,
        stores,
        retained_sources,
        choice,
        selected_path,
        source_root,
        manifest,
        coverage,
        retained_generation,
        source_lease,
    })
}

pub(super) fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(crate::SessionOperationSettlement::RoutineCancellation.into());
    }
    Ok(())
}
