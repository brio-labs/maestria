use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use sillage_daemon::{SessionPreparationOwner, SessionSearchCancellation, SessionSearchRuntime};
use sillage_domain::{
    ControlPersistenceChoice, DocumentaryRetentionChoice, SourceChoice, SourceScope,
};
use tokio::sync::{Notify, oneshot, watch};

use crate::application::passages::SessionEvidenceBinding;

mod lifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourcePhase {
    Unselected,
    Selected,
    Preparing,
    Ready,
    Partial,
    Searching,
    Results,
    Refused,
    Error,
    Cancelled,
    Retiring,
    Stopped,
}

impl SourcePhase {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Unselected => "No Markdown file selected",
            Self::Selected => "Markdown file selected; not prepared",
            Self::Preparing => "Preparing selected Markdown file",
            Self::Ready => "Ready for on-demand search",
            Self::Partial => "Prepared with partial coverage",
            Self::Searching => "Searching the selected Markdown file",
            Self::Results => "Current scoped search results",
            Self::Refused => "Search or evidence was refused",
            Self::Error => "Source operation failed",
            Self::Cancelled => "Source operation cancelled",
            Self::Retiring => "Retiring the previous source",
            Self::Stopped => "Source session stopped",
        }
    }
}

#[derive(Clone)]
pub(super) struct SourceQuery {
    pub(super) query: String,
    pub(super) generation: u64,
    pub(super) epoch: u64,
    pub(super) request_id: u64,
    pub(super) cancellation: SessionSearchCancellation,
    pub(super) catalog_applied: Arc<tokio::sync::Mutex<Option<oneshot::Receiver<()>>>>,
}

pub(super) struct SourceSnapshot {
    pub(super) selected_path: Option<PathBuf>,
    pub(super) consent: bool,
    pub(super) documentary_retention: DocumentaryRetentionChoice,
    pub(super) control_persistence: ControlPersistenceChoice,
    pub(super) phase: SourcePhase,
    pub(super) status: String,
    pub(super) detail: String,
    pub(super) error: String,
    pub(super) busy: bool,
    pub(super) can_prepare: bool,
    pub(super) can_search: bool,
    pub(super) can_purge: bool,
}

pub(super) struct ProofLease<'a> {
    pub(super) runtime: Arc<SessionSearchRuntime>,
    pub(super) cancellation: SessionSearchCancellation,
    pub(super) terminal_slot: TerminalOperationReservation<'a>,
}

struct ActiveSession {
    owner: Arc<SessionPreparationOwner>,
    runtime: Arc<SessionSearchRuntime>,
    choice: SourceChoice,
}

struct SessionState {
    selected_path: Option<PathBuf>,
    consent: bool,
    documentary_retention: DocumentaryRetentionChoice,
    control_persistence: ControlPersistenceChoice,
    phase: SourcePhase,
    status: String,
    detail: String,
    error: String,
    picker_busy: bool,
    active: Option<ActiveSession>,
    prepare_cancellation: Option<SessionSearchCancellation>,
    query_cancellation: Option<SessionSearchCancellation>,
    proof_cancellation: Option<SessionSearchCancellation>,
    query_running: bool,
    proof_running: bool,
    epoch: u64,
    query_generation: u64,
    next_request_id: u64,
    current_request_id: Option<u64>,
    source_revision: Option<i64>,
    terminal_operation_failures: Vec<String>,
    reserved_terminal_operation_slots: usize,
}

const MAX_TERMINAL_OPERATION_FAILURES: usize = 16;

pub(super) struct TerminalOperationReservation<'a> {
    state: &'a SessionSourceState,
    active: bool,
}

impl TerminalOperationReservation<'_> {
    pub(super) fn record(mut self, error: String) {
        let mut state = lock(&self.state.inner);
        state.reserved_terminal_operation_slots -= 1;
        state.terminal_operation_failures.push(error);
        self.active = false;
    }
}

impl Drop for TerminalOperationReservation<'_> {
    fn drop(&mut self) {
        if self.active {
            let mut state = lock(&self.state.inner);
            state.reserved_terminal_operation_slots -= 1;
            self.active = false;
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LifecycleTestStage {
    RemoveAfterSnapshot,
    RemoveAfterSettingsWrite,
    QueryBeforeAtomicFailureSettlement,
    PrepareBeforeAtomicFailureSettlement,
    ProofOpenViewAfterReopen,
    ProofActionAfterReopen,
}

#[cfg(test)]
pub(super) struct LifecycleTestObservation {
    pub(super) failed: bool,
    pub(super) original_error: Option<String>,
}

#[cfg(test)]
type LifecycleTestGate = (
    LifecycleTestStage,
    std::sync::mpsc::SyncSender<LifecycleTestObservation>,
    std::sync::mpsc::Receiver<()>,
);

pub(in crate::application) struct SessionSourceState {
    inner: Mutex<SessionState>,
    control_metadata_transition: Mutex<()>,
    changes: watch::Sender<u64>,
    queries: watch::Sender<Option<SourceQuery>>,
    operations_changed: Notify,
    stopping: AtomicBool,
    #[cfg(test)]
    prepare_test_gate: Mutex<Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>>,
    #[cfg(test)]
    lifecycle_test_gate: Mutex<Option<LifecycleTestGate>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl Default for SessionSourceState {
    fn default() -> Self {
        let (changes, _) = watch::channel(0);
        let (queries, _) = watch::channel(None);
        Self {
            inner: Mutex::new(SessionState {
                selected_path: None,
                consent: false,
                documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
                control_persistence: ControlPersistenceChoice::SessionOnly,
                phase: SourcePhase::Unselected,
                status: "Choose one local Markdown file".to_string(),
                detail: "Selection alone reads no source content and starts no preparation."
                    .to_string(),
                error: String::new(),
                picker_busy: false,
                active: None,
                prepare_cancellation: None,
                query_cancellation: None,
                proof_cancellation: None,
                query_running: false,
                proof_running: false,
                epoch: 0,
                query_generation: 0,
                next_request_id: 0,
                current_request_id: None,
                source_revision: None,
                terminal_operation_failures: Vec::with_capacity(MAX_TERMINAL_OPERATION_FAILURES),
                reserved_terminal_operation_slots: 0,
            }),
            control_metadata_transition: Mutex::new(()),
            changes,
            queries,
            operations_changed: Notify::new(),
            #[cfg(test)]
            prepare_test_gate: Mutex::new(None),
            #[cfg(test)]
            lifecycle_test_gate: Mutex::new(None),
            stopping: AtomicBool::new(false),
        }
    }
}

impl SessionSourceState {
    pub(super) fn control_metadata_transition(&self) -> MutexGuard<'_, ()> {
        lock(&self.control_metadata_transition)
    }
    #[cfg(test)]
    pub(super) fn control_metadata_transition_is_locked(&self) -> bool {
        self.control_metadata_transition.try_lock().is_err()
    }

    #[cfg(test)]
    pub(super) fn install_lifecycle_test_gate(
        &self,
        stage: LifecycleTestStage,
        entered: std::sync::mpsc::SyncSender<LifecycleTestObservation>,
        release: std::sync::mpsc::Receiver<()>,
    ) {
        *lock(&self.lifecycle_test_gate) = Some((stage, entered, release));
    }

    #[cfg(test)]
    pub(super) fn wait_at_lifecycle_test_gate(
        &self,
        stage: LifecycleTestStage,
        failed: bool,
        original_error: Option<String>,
    ) {
        let gate = {
            let mut gate = lock(&self.lifecycle_test_gate);
            if gate
                .as_ref()
                .is_some_and(|(candidate, _, _)| *candidate == stage)
            {
                gate.take()
            } else {
                None
            }
        };
        if let Some((_, entered, release)) = gate {
            let _ = entered.send(LifecycleTestObservation {
                failed,
                original_error,
            });
            let _ = release.recv();
        }
    }
    #[cfg(test)]
    pub(super) fn install_prepare_test_gate(
        &self,
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    ) {
        *lock(&self.prepare_test_gate) = Some((entered, release));
    }

    #[cfg(test)]
    pub(super) async fn wait_at_prepare_test_gate(&self) {
        let Some((entered, release)) = lock(&self.prepare_test_gate).take() else {
            return;
        };
        let _ = entered.send(());
        let _ = release.await;
    }

    pub(super) fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(super) fn subscribe_queries(&self) -> watch::Receiver<Option<SourceQuery>> {
        self.queries.subscribe()
    }

    fn notify_changed(&self) {
        self.changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub(super) fn snapshot(&self) -> SourceSnapshot {
        let state = lock(&self.inner);
        let busy = state.picker_busy
            || matches!(
                state.phase,
                SourcePhase::Preparing | SourcePhase::Searching | SourcePhase::Retiring
            )
            || state.proof_running;
        SourceSnapshot {
            selected_path: state.selected_path.clone(),
            consent: state.consent,
            documentary_retention: state.documentary_retention,
            control_persistence: state.control_persistence,
            phase: state.phase,
            status: state.status.clone(),
            detail: state.detail.clone(),
            error: state.error.clone(),
            busy,
            can_prepare: state.selected_path.is_some()
                && state.consent
                && state.active.is_none()
                && !state.picker_busy
                && !self.stopping.load(Ordering::Acquire),
            can_search: state.consent
                && state.active.is_some()
                && matches!(
                    state.phase,
                    SourcePhase::Ready | SourcePhase::Partial | SourcePhase::Results
                ),
            can_purge: state.selected_path.is_some()
                && !state.picker_busy
                && !matches!(state.phase, SourcePhase::Preparing | SourcePhase::Retiring),
        }
    }

    pub(in crate::application) fn is_selected(&self) -> bool {
        let state = lock(&self.inner);
        state.selected_path.is_some() || state.phase == SourcePhase::Retiring
    }

    pub(super) fn selected_path(&self) -> Option<PathBuf> {
        lock(&self.inner).selected_path.clone()
    }

    pub(super) fn restore_controls(
        &self,
        documentary_retention: DocumentaryRetentionChoice,
        control_persistence: ControlPersistenceChoice,
        selected_scope: Option<SourceScope>,
    ) {
        let mut state = lock(&self.inner);
        state.documentary_retention = documentary_retention;
        state.control_persistence = control_persistence;
        if let Some(scope) = selected_scope
            .filter(|_| control_persistence == ControlPersistenceChoice::PersistExplicitly)
        {
            state.selected_path = Some(scope.path().to_path_buf());
            state.phase = SourcePhase::Selected;
            state.status = "Remembered Markdown source control".to_string();
            state.detail = "This path is not read or prepared until you re-authorize reading and press Prepare.".to_string();
        }
        drop(state);
        self.notify_changed();
    }
    pub(super) async fn wait_until_stopping(&self) {
        let mut changes = self.subscribe_changes();
        while !self.is_stopping() {
            if changes.changed().await.is_err() {
                return;
            }
        }
    }

    pub(super) fn begin_picker(&self) -> bool {
        let mut state = lock(&self.inner);
        if self.stopping.load(Ordering::Acquire) || state.picker_busy {
            return false;
        }
        state.picker_busy = true;
        state.error.clear();
        drop(state);
        self.notify_changed();
        true
    }

    pub(super) fn finish_picker(&self) {
        lock(&self.inner).picker_busy = false;
        self.notify_changed();
    }

    pub(super) fn set_consent(&self, consent: bool) {
        if !consent {
            self.revoke_consent();
            return;
        }
        let mut state = lock(&self.inner);
        state.consent = state.selected_path.is_some() && !self.stopping.load(Ordering::Acquire);
        state.error.clear();
        drop(state);
        self.notify_changed();
    }

    pub(super) fn revoke_consent(&self) {
        let mut state = lock(&self.inner);
        state.consent = false;
        state.epoch = state.epoch.saturating_add(1);
        state.current_request_id = None;
        state.source_revision = None;
        for cancellation in [
            state.prepare_cancellation.take(),
            state.query_cancellation.take(),
            state.proof_cancellation.take(),
        ]
        .into_iter()
        .flatten()
        {
            cancellation.cancel();
        }
        state.phase = if state.active.is_some() || state.phase == SourcePhase::Preparing {
            SourcePhase::Retiring
        } else if state.selected_path.is_some() {
            SourcePhase::Selected
        } else {
            SourcePhase::Unselected
        };
        state.status = "Read authorization withdrawn; retiring selected-source work".to_string();
        state.detail.clear();
        state.error.clear();
        drop(state);
        self.queries.send_replace(None);
        self.notify_changed();
        self.operations_changed.notify_waiters();
    }

    pub(super) fn set_controls(
        &self,
        documentary_retention: DocumentaryRetentionChoice,
        control_persistence: ControlPersistenceChoice,
    ) -> Result<(), String> {
        let mut state = lock(&self.inner);
        if documentary_retention != state.documentary_retention
            && (state.active.is_some()
                || matches!(state.phase, SourcePhase::Preparing | SourcePhase::Retiring))
        {
            return Err(
                "Retire or purge the prepared source before changing documentary retention"
                    .to_string(),
            );
        }
        state.documentary_retention = documentary_retention;
        state.control_persistence = control_persistence;
        state.error.clear();
        drop(state);
        self.notify_changed();
        Ok(())
    }

    pub(super) fn set_selection(&self, path: PathBuf) {
        let mut state = lock(&self.inner);
        state.epoch = state.epoch.saturating_add(1);
        state.selected_path = Some(path);
        state.consent = false;
        state.active = None;
        state.prepare_cancellation = None;
        state.query_cancellation = None;
        state.proof_cancellation = None;
        state.current_request_id = None;
        state.source_revision = None;
        state.phase = SourcePhase::Selected;
        state.status = "Markdown file selected".to_string();
        state.detail = "Exact file scope only. Check read consent, then explicitly Prepare; no parent folder is approved.".to_string();
        state.error.clear();
        drop(state);
        self.notify_changed();
    }

    pub(super) fn begin_prepare(
        &self,
    ) -> Result<
        (
            u64,
            PathBuf,
            DocumentaryRetentionChoice,
            ControlPersistenceChoice,
            SessionSearchCancellation,
            TerminalOperationReservation<'_>,
        ),
        String,
    > {
        let terminal_slot = self.reserve_terminal_operation()?;
        let mut state = lock(&self.inner);
        if self.stopping.load(Ordering::Acquire) {
            return Err("The launcher is shutting down".to_string());
        }
        let path = state
            .selected_path
            .clone()
            .ok_or_else(|| "Choose one Markdown file before preparing".to_string())?;
        if !state.consent {
            return Err(
                "Explicitly authorize reading this selected file before preparation".to_string(),
            );
        }
        if state.active.is_some() || state.phase == SourcePhase::Preparing {
            return Err("The selected source is already prepared or being prepared".to_string());
        }
        let cancellation = SessionSearchCancellation::new();
        state.epoch = state.epoch.saturating_add(1);
        let epoch = state.epoch;
        state.prepare_cancellation = Some(cancellation.clone());
        state.phase = SourcePhase::Preparing;
        state.status = "Preparing only the selected Markdown file".to_string();
        state.detail =
            "OnDemand preparation starts only after this gesture; no model is downloaded."
                .to_string();
        state.error.clear();
        let retention = state.documentary_retention;
        let persistence = state.control_persistence;
        drop(state);
        self.notify_changed();
        Ok((
            epoch,
            path,
            retention,
            persistence,
            cancellation,
            terminal_slot,
        ))
    }

    pub(super) fn cancel_prepare(&self) -> bool {
        let cancelled = {
            let state = lock(&self.inner);
            state
                .prepare_cancellation
                .as_ref()
                .is_some_and(|cancellation| {
                    cancellation.cancel();
                    true
                })
        };
        if cancelled {
            self.notify_changed();
        }
        cancelled
    }

    pub(super) fn finish_prepare(
        &self,
        epoch: u64,
        cancellation: &SessionSearchCancellation,
        owner: Arc<SessionPreparationOwner>,
        runtime: Arc<SessionSearchRuntime>,
        choice: SourceChoice,
    ) -> bool {
        let coverage_complete = runtime.coverage().coverage_complete;
        let mut state = lock(&self.inner);
        if state.epoch != epoch
            || self.stopping.load(Ordering::Acquire)
            || cancellation.is_cancelled()
        {
            return false;
        }
        state.active = Some(ActiveSession {
            owner,
            runtime,
            choice,
        });
        state.prepare_cancellation = None;
        state.phase = if coverage_complete {
            SourcePhase::Ready
        } else {
            SourcePhase::Partial
        };
        state.status = if coverage_complete {
            "Prepared; source coverage reported complete within the selected file".to_string()
        } else {
            "Prepared with partial coverage; conclusions are limited to examined data".to_string()
        };
        state.detail = "Representation: local lexical retrieval. The configured 67,108,864-byte logical admission quota is not an RSS or physical-memory cap.".to_string();
        state.error.clear();
        drop(state);
        self.notify_changed();
        true
    }

    pub(super) fn reserve_terminal_operation(
        &self,
    ) -> Result<TerminalOperationReservation<'_>, String> {
        let mut state = lock(&self.inner);
        if state.terminal_operation_failures.len() + state.reserved_terminal_operation_slots
            >= MAX_TERMINAL_OPERATION_FAILURES
        {
            return Err(
                "Terminal source failures are retained; restart the launcher before more work."
                    .to_string(),
            );
        }
        state.reserved_terminal_operation_slots += 1;
        Ok(TerminalOperationReservation {
            state: self,
            active: true,
        })
    }

    pub(super) fn take_terminal_operation_failures(&self) -> Vec<String> {
        std::mem::take(&mut lock(&self.inner).terminal_operation_failures)
    }

    pub(in crate::application) fn query_changed(&self, generation: u64) {
        let mut state = lock(&self.inner);
        if let Some(cancellation) = state.query_cancellation.take() {
            cancellation.cancel();
        }
        if let Some(cancellation) = state.proof_cancellation.take() {
            cancellation.cancel();
        }
        state.query_generation = generation;
        if matches!(state.phase, SourcePhase::Searching | SourcePhase::Results) {
            state.phase = if state
                .active
                .as_ref()
                .is_some_and(|active| active.runtime.coverage().coverage_complete)
            {
                SourcePhase::Ready
            } else if state.active.is_some() {
                SourcePhase::Partial
            } else {
                SourcePhase::Selected
            };
        }
        state.status = match state.phase {
            SourcePhase::Ready | SourcePhase::Partial => {
                "Enter a query to search only the prepared Markdown file".to_string()
            }
            SourcePhase::Selected => {
                "Selected Markdown file is not prepared; press Prepare before searching".to_string()
            }
            _ => state.phase.label().to_string(),
        };
        state.current_request_id = None;
        drop(state);
        self.queries.send_replace(None);
        self.notify_changed();
        self.operations_changed.notify_waiters();
    }

    pub(in crate::application) fn submit_query(
        &self,
        query: String,
        generation: u64,
        catalog_applied: oneshot::Receiver<()>,
    ) -> Result<(), String> {
        let mut state = lock(&self.inner);
        if self.stopping.load(Ordering::Acquire) {
            return Err("The launcher is shutting down".to_string());
        }
        if state.query_generation != generation {
            return Err("This query belongs to an older foreground generation".to_string());
        }
        if query.is_empty() {
            return Ok(());
        }
        if state.phase == SourcePhase::Retiring {
            return Err("The selected source is retiring and cannot accept queries".to_string());
        }
        if !state.consent || state.active.is_none() {
            return Err(
                "Re-authorize reading and prepare the selected Markdown file before searching"
                    .to_string(),
            );
        }
        let query_id = state.next_request_id.checked_add(1).ok_or_else(|| {
            "The source request sequence is exhausted; restart the launcher".to_string()
        })?;
        state.next_request_id = query_id;
        if let Some(cancellation) = state.query_cancellation.take() {
            cancellation.cancel();
        }
        if let Some(cancellation) = state.proof_cancellation.take() {
            cancellation.cancel();
        }
        let cancellation = SessionSearchCancellation::new();
        state.query_cancellation = Some(cancellation.clone());
        state.current_request_id = Some(query_id);
        state.phase = SourcePhase::Searching;
        state.status = "Searching within the selected Markdown file".to_string();
        state.detail.clear();
        state.error.clear();
        let request = SourceQuery {
            query,
            generation,
            epoch: state.epoch,
            request_id: query_id,
            cancellation,
            catalog_applied: Arc::new(tokio::sync::Mutex::new(Some(catalog_applied))),
        };
        drop(state);
        self.queries.send_replace(Some(request));
        self.notify_changed();
        Ok(())
    }

    pub(super) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub(super) fn current_runtime_for_query(
        &self,
        request: &SourceQuery,
    ) -> Option<(Arc<SessionSearchRuntime>, PathBuf, SourceChoice)> {
        let state = lock(&self.inner);
        if state.epoch != request.epoch
            || state.query_generation != request.generation
            || state.current_request_id != Some(request.request_id)
            || request.cancellation.is_cancelled()
        {
            return None;
        }
        let active = state.active.as_ref()?;
        Some((
            Arc::clone(&active.runtime),
            state.selected_path.clone()?,
            active.choice.clone(),
        ))
    }

    pub(super) fn begin_query_operation(
        &self,
        request: &SourceQuery,
    ) -> Result<Option<TerminalOperationReservation<'_>>, String> {
        {
            let state = lock(&self.inner);
            if self.stopping.load(Ordering::Acquire)
                || state.phase == SourcePhase::Retiring
                || state.epoch != request.epoch
                || state.query_generation != request.generation
                || state.current_request_id != Some(request.request_id)
            {
                return Ok(None);
            }
        }
        let terminal_slot = self.reserve_terminal_operation()?;
        let mut state = lock(&self.inner);
        if self.stopping.load(Ordering::Acquire)
            || state.phase == SourcePhase::Retiring
            || state.epoch != request.epoch
            || state.query_generation != request.generation
            || state.current_request_id != Some(request.request_id)
            || state.query_running
        {
            return Ok(None);
        }
        state.query_running = true;
        Ok(Some(terminal_slot))
    }

    pub(super) fn finish_query_operation(&self, _request: &SourceQuery) {
        lock(&self.inner).query_running = false;
        self.operations_changed.notify_waiters();
    }

    pub(super) fn query_is_current(&self, request: &SourceQuery) -> bool {
        let state = lock(&self.inner);
        !self.stopping.load(Ordering::Acquire)
            && state.epoch == request.epoch
            && state.query_generation == request.generation
            && state.current_request_id == Some(request.request_id)
            && !request.cancellation.is_cancelled()
    }
    pub(super) fn query_request_is_current(&self, request: &SourceQuery) -> bool {
        let state = lock(&self.inner);
        state.epoch == request.epoch
            && state.query_generation == request.generation
            && state.current_request_id == Some(request.request_id)
    }

    pub(super) fn finish_query_with_detail(
        &self,
        request: &SourceQuery,
        revision: i64,
        phase: SourcePhase,
        status: String,
        detail: String,
    ) {
        let mut state = lock(&self.inner);
        if state.epoch != request.epoch
            || state.query_generation != request.generation
            || state.current_request_id != Some(request.request_id)
            || request.cancellation.is_cancelled()
        {
            return;
        }
        state.source_revision = Some(revision);
        state.phase = phase;
        state.status = status;
        state.detail = detail;
        state.error.clear();
        state.query_cancellation = None;
        drop(state);
        self.notify_changed();
    }

    pub(super) fn fail_query(&self, request: &SourceQuery, error: String, refused: bool) {
        let mut state = lock(&self.inner);
        if state.epoch != request.epoch
            || state.query_generation != request.generation
            || state.current_request_id != Some(request.request_id)
        {
            return;
        }
        if self.stopping.load(Ordering::Acquire)
            || matches!(
                state.phase,
                SourcePhase::Preparing | SourcePhase::Retiring | SourcePhase::Stopped
            )
        {
            state.status = "Selected-source operation failed".to_string();
        } else {
            state.phase = if refused {
                SourcePhase::Refused
            } else {
                SourcePhase::Error
            };
            state.status = state.phase.label().to_string();
        }
        state.error = error;
        state.query_cancellation = None;
        drop(state);
        self.notify_changed();
    }

    pub(super) fn begin_proof(
        &self,
        binding: &SessionEvidenceBinding,
    ) -> Result<ProofLease<'_>, String> {
        let terminal_slot = self.reserve_terminal_operation()?;
        let mut state = lock(&self.inner);
        if self.stopping.load(Ordering::Acquire)
            || state.phase == SourcePhase::Retiring
            || state.epoch != binding.session_epoch
            || state.query_generation != binding.query_generation
            || state.current_request_id != Some(binding.request_id)
            || state.source_revision != Some(binding.source_revision)
        {
            return Err(
                "This evidence belongs to an older or retired selected-source session".to_string(),
            );
        }
        if state.proof_running {
            return Err("A current-evidence proof operation is already running".to_string());
        }
        let runtime = state
            .active
            .as_ref()
            .map(|active| Arc::clone(&active.runtime))
            .ok_or_else(|| "The selected source session is no longer available".to_string())?;
        if state.selected_path.is_none() {
            return Err("The selected source path is no longer available".to_string());
        }
        let cancellation = SessionSearchCancellation::new();
        state.proof_cancellation = Some(cancellation.clone());
        state.proof_running = true;
        Ok(ProofLease {
            runtime,
            cancellation,
            terminal_slot,
        })
    }

    pub(super) fn finish_proof(&self) {
        let mut state = lock(&self.inner);
        state.proof_running = false;
        state.proof_cancellation = None;
        drop(state);
        self.operations_changed.notify_waiters();
    }

    pub(super) fn binding_is_current(&self, binding: &SessionEvidenceBinding) -> bool {
        let state = lock(&self.inner);
        !self.stopping.load(Ordering::Acquire)
            && state.epoch == binding.session_epoch
            && state.query_generation == binding.query_generation
            && state.current_request_id == Some(binding.request_id)
            && state.source_revision == Some(binding.source_revision)
            && state.selected_path.as_ref() == Some(&binding.source_path)
    }
}
#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
