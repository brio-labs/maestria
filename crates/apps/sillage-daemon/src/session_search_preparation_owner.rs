use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

use anyhow::{Result, anyhow, bail};
use sillage_domain::SourceChoice;
use sillage_search_tantivy::RamMemoryBudget;
use tokio_util::sync::CancellationToken;

use super::{
    SessionSearchRuntime,
    control::SessionControl,
    operations::OperationTracker,
    retained,
    source_admission::{SourceLease, validate_source_path, with_purge_exclusion},
};

struct OwnerState {
    retention_root: Option<Arc<PathBuf>>,
    operations: Arc<OperationTracker>,
    controls: Mutex<Vec<Weak<SessionControl>>>,
    retirement_failures: tokio::sync::Mutex<Vec<anyhow::Error>>,
}

/// Caller-retained owner for preparation, retirement, and explicit source purge.
#[derive(Clone)]
pub struct SessionPreparationOwner {
    state: Arc<OwnerState>,
}

impl Default for SessionPreparationOwner {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionPreparationOwner {
    /// Creates an owner without granting documentary persistence.
    pub fn new() -> Self {
        Self::with_root(None)
    }

    /// Grants an explicit absolute root for retained documentary generations.
    /// This validates the path only; no filesystem access occurs before Prepare.
    pub fn with_retention_root(root: PathBuf) -> Result<Self> {
        validate_retention_root(&root)?;
        Ok(Self::with_root(Some(Arc::new(root))))
    }

    fn with_root(retention_root: Option<Arc<PathBuf>>) -> Self {
        Self {
            state: Arc::new(OwnerState {
                retention_root,
                operations: Arc::new(OperationTracker::new()),
                controls: Mutex::new(Vec::new()),
                retirement_failures: tokio::sync::Mutex::new(Vec::new()),
            }),
        }
    }

    /// Prepares one explicitly selected local Markdown file.
    ///
    /// # Cancellation
    ///
    /// Dropping this future requests cancellation; the owner retains and joins the accepted
    /// operation. Cancellation and deadline outcomes retain typed settlement and the original
    /// worker/parser cause, including when retirement later reports a dropped caller's error.
    pub async fn prepare_cancellable(
        &self,
        choice: SourceChoice,
        cancellation: CancellationToken,
    ) -> Result<SessionSearchRuntime> {
        super::validate_session_choice(&choice, self.state.retention_root.is_some())?;
        let memory = Arc::new(RamMemoryBudget::new(choice.budget().max_in_memory_bytes()));
        let selected_path = choice.scope().path();
        validate_source_path(selected_path)?;
        let source_lease = SourceLease::acquire(selected_path, choice.availability(), &memory)?;
        let control = source_lease.control();
        self.remember_control(&control)?;
        let retention_root = if choice.documentary_retention()
            == sillage_domain::DocumentaryRetentionChoice::RetainUntilExplicitlyPurged
        {
            self.state.retention_root.clone()
        } else {
            None
        };
        let control_cancellation = cancellation.clone();
        let worker_token = cancellation.clone();
        let owner = self.state.operations.clone();
        let admission = owner.clone();
        owner
            .track(cancellation, async move {
                control
                    .track(control_cancellation, async move {
                        let mut runtime = SessionSearchRuntime::prepare_native(
                            choice,
                            worker_token,
                            retention_root,
                            source_lease,
                            memory,
                        )
                        .await?;
                        admission.with_open_admission(|| runtime.commit_retention())?;
                        Ok(runtime)
                    })
                    .await
            })
            .await
    }

    /// Waits for accepted preparations and operations; it does not cancel them. Already-requested
    /// session retirement cleanup is joined before this returns.
    ///
    /// # Cancellation
    ///
    /// Dropping this wait affects only the caller's wait. It does not cancel accepted operations or
    /// consume owner retirement errors; cleanup remains in the owning session.
    pub async fn wait_for_idle(&self) {
        self.state.operations.wait().await;
        let controls = self.live_controls();
        for control in controls {
            control.wait_for_idle().await;
        }
    }

    /// Stops preparation/search admission and starts session-owned cleanup.
    ///
    /// # Cancellation
    ///
    /// Dropping this future stops only this wait; cleanup continues. Owner and control errors
    /// remain stored until one `retire` call returns them exactly once. `wait_for_idle` joins
    /// cleanup without consuming those errors, and retirement never purges retained generations.
    pub async fn retire(&self) -> Result<()> {
        let mut failures = self.state.retirement_failures.lock().await;
        self.state.operations.close_and_cancel();
        let controls = self.live_controls();
        for control in &controls {
            control.request_retirement();
        }
        failures.extend(self.state.operations.wait_and_collect_errors().await);
        for control in controls {
            if let Err(error) = control.retire().await {
                failures.push(error);
            }
        }
        super::operations::errors_result(std::mem::take(&mut *failures))
    }

    /// Explicitly removes only generations owned for this selected source.
    /// The source must be fully retired; unknown namespaces are never adopted or removed.
    pub fn purge_retained_source(&self, source_path: &Path) -> Result<()> {
        let root = self
            .state
            .retention_root
            .as_ref()
            .ok_or_else(|| anyhow!("documentary purge requires an explicit retention root"))?;
        validate_source_path(source_path)?;
        with_purge_exclusion(source_path, || {
            retained::purge_source(root.as_path(), source_path)
        })
    }

    fn remember_control(&self, control: &Arc<SessionControl>) -> Result<()> {
        let mut controls = self
            .state
            .controls
            .lock()
            .map_err(|_| super::SessionLifecycleFailure::OwnerControlRegistry)?;
        controls.retain(|control| control.strong_count() != 0);
        controls.try_reserve(1)?;
        controls.push(Arc::downgrade(control));
        Ok(())
    }

    fn live_controls(&self) -> Vec<Arc<SessionControl>> {
        let (mut controls, poisoned) = match self.state.controls.lock() {
            Ok(controls) => (controls, false),
            Err(poisoned) => (poisoned.into_inner(), true),
        };
        controls.retain(|control| control.strong_count() != 0);
        let live = controls.iter().filter_map(Weak::upgrade).collect();
        drop(controls);
        if poisoned {
            self.state.operations.record_owner_registry_poison();
        }
        live
    }
}

fn validate_retention_root(root: &Path) -> Result<()> {
    if !root.is_absolute()
        || root.as_os_str().is_empty()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        bail!("retention store root must be absolute and normalized");
    }
    Ok(())
}
