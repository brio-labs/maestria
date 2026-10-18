use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

pub(super) struct PreviewTestGate {
    arrival: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: Mutex<bool>,
    wake: Condvar,
}

impl PreviewTestGate {
    fn new() -> (Arc<Self>, tokio::sync::oneshot::Receiver<()>) {
        let (arrival, reached) = tokio::sync::oneshot::channel();
        (
            Arc::new(Self {
                arrival: Mutex::new(Some(arrival)),
                released: Mutex::new(false),
                wake: Condvar::new(),
            }),
            reached,
        )
    }

    fn pause(&self) {
        let arrival = lock(&self.arrival).take();
        if let Some(arrival) = arrival
            && arrival.send(()).is_err()
        {
            self.release();
            return;
        }
        let mut released = lock(&self.released);
        while !*released {
            released = match self.wake.wait(released) {
                Ok(released) => released,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }

    fn release(&self) {
        let mut released = lock(&self.released);
        *released = true;
        self.wake.notify_all();
    }
}

static PREVIEW_RESULT_GATE: Mutex<Option<Arc<PreviewTestGate>>> = Mutex::new(None);
static PREVIEW_GATE_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) fn install_preview_result_gate(gate: Arc<PreviewTestGate>) -> Result<()> {
    let mut current = lock(&PREVIEW_RESULT_GATE);
    if current.is_some() {
        bail!("a preview result test gate is already installed");
    }
    *current = Some(gate);
    Ok(())
}

pub(super) fn pause_preview_result_gate() {
    if let Some(gate) = lock(&PREVIEW_RESULT_GATE).take() {
        gate.pause();
    }
}

pub(super) fn clear_preview_result_gate(expected: &Arc<PreviewTestGate>) {
    let mut current = lock(&PREVIEW_RESULT_GATE);
    if current
        .as_ref()
        .is_some_and(|installed| Arc::ptr_eq(installed, expected))
    {
        current.take();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[tokio::test]
async fn cancellation_after_independent_preview_failure_preserves_typed_worker_error() -> Result<()>
{
    let _serial = PREVIEW_GATE_SERIAL.lock().await;
    let (root, source) = create_workspace()?;
    let owner = crate::SessionPreparationOwner::new();
    let choice = SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source.clone())?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })?;
    let runtime = Arc::new(
        owner
            .prepare_cancellable(choice, crate::SessionSearchCancellation::new())
            .await
            .context("cannot prepare the selected Markdown fixture")?,
    );
    let response = runtime
        .search("saffron comet".to_string(), 8)
        .await
        .context("cannot establish the original selected-source citation")?;
    let citation = response
        .previews
        .first()
        .ok_or_else(|| anyhow!("selected-source fixture produced no citation"))?
        .clone();
    fs::write(&source, "# Notes\n\nThe notebook changed after citation.\n")
        .context("cannot mutate selected source for independent stale failure")?;

    let (gate, reached_signal) = PreviewTestGate::new();
    install_preview_result_gate(gate.clone())?;
    let cancellation = crate::SessionSearchCancellation::new();
    let worker_runtime = runtime.clone();
    let worker_cancellation = cancellation.clone();
    let worker = tokio::spawn(async move {
        worker_runtime
            .preview_evidence(
                citation.evidence_id,
                citation.artifact_version,
                citation.source_revision,
                worker_cancellation,
            )
            .await
    });
    let reached = tokio::time::timeout(Duration::from_secs(10), reached_signal)
        .await
        .is_ok_and(|result| result.is_ok());
    cancellation.cancel();
    gate.release();
    let worker_result = worker
        .await
        .map_err(anyhow::Error::new)
        .context("preview caller task failed to join");
    clear_preview_result_gate(&gate);
    let runtime_retirement = runtime.retire().await;
    let owner_retirement = owner.retire().await;
    let cleanup = fs::remove_dir_all(&root).context("cannot remove preview test workspace");

    if !reached {
        return Err(anyhow!(
            "preview worker did not reach the post-result phase gate"
        ));
    }
    runtime_retirement.context("runtime retirement failed")?;
    owner_retirement.context("owner retirement failed")?;
    cleanup?;
    match worker_result? {
        Err(error)
            if error
                .downcast_ref::<crate::session_search::StaleEvidence>()
                .is_some() =>
        {
            Ok(())
        }
        Err(error) => Err(error.context("settled preview failure lost its typed stale cause")),
        Ok(_) => bail!("changed selected source produced a preview after cancellation"),
    }
}

#[path = "session_search_preview_lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(unix)]
#[path = "session_search_preview_native_tests.rs"]
mod native_tests;

fn create_workspace() -> Result<(PathBuf, PathBuf)> {
    let parent = std::env::temp_dir();
    for _ in 0..128 {
        let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = parent.join(format!(
            ".sillage-preview-settlement-test-{}-{sequence}",
            std::process::id()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&root) {
            Ok(()) => {
                let source = root.join("selected.md");
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                options
                    .open(&source)?
                    .write_all(b"# Notes\n\nThe saffron comet appears here.\n")?;
                return Ok((root, source));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("cannot allocate an exclusive preview settlement workspace")
}

struct PreviewWorkspaceGuard {
    root: PathBuf,
    armed: bool,
}

impl PreviewWorkspaceGuard {
    fn new(root: PathBuf) -> Self {
        Self { root, armed: true }
    }

    fn cleanup(mut self) -> Result<()> {
        self.armed = false;
        fs::remove_dir_all(&self.root).context("cannot remove preview test workspace")
    }
}

impl Drop for PreviewWorkspaceGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn selected_choice(
    source: PathBuf,
    documentary_retention: DocumentaryRetentionChoice,
) -> Result<SourceChoice> {
    SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source)?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })
    .map_err(anyhow::Error::new)
}
