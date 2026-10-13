use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};

use crate::{SessionOperationSettlement, SessionPreparationOwner, SessionSearchCancellation};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);
const SOURCE_LINE: &[u8] = b"The amber satellite follows a stable coastal orbit.\n";
const SOURCE_LINE_COUNT: usize = 2048;

struct Workspace(PathBuf);

impl Workspace {
    fn create() -> Result<(Self, PathBuf)> {
        let parent = std::env::temp_dir();
        for _ in 0..128 {
            let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
            let root = parent.join(format!(
                ".sillage-parse-cancellation-test-{}-{sequence}",
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
                    let mut file = options
                        .open(&source)
                        .context("cannot create private parser-cancellation fixture")?;
                    for _ in 0..SOURCE_LINE_COUNT {
                        file.write_all(SOURCE_LINE)?;
                    }
                    return Ok((Self(root), source));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).context("cannot create parser-cancellation workspace");
                }
            }
        }
        bail!("cannot allocate a private parser-cancellation workspace")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(in crate::session_search) struct ParseCheckpointGate {
    target_path: PathBuf,
    arrival: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: Mutex<bool>,
    wake: Condvar,
}

impl ParseCheckpointGate {
    pub(in crate::session_search) fn new(
        target_path: PathBuf,
    ) -> (Arc<Self>, tokio::sync::oneshot::Receiver<()>) {
        let (arrival, reached) = tokio::sync::oneshot::channel();
        (
            Arc::new(Self {
                target_path,
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

    pub(in crate::session_search) fn release(&self) {
        let mut released = lock(&self.released);
        *released = true;
        self.wake.notify_all();
    }
}

static PARSE_CHECKPOINT_GATE: Mutex<Option<Arc<ParseCheckpointGate>>> = Mutex::new(None);
pub(in crate::session_search) static PARSE_TEST_SERIAL: tokio::sync::Mutex<()> =
    tokio::sync::Mutex::const_new(());

pub(in crate::session_search) fn install_gate(gate: Arc<ParseCheckpointGate>) -> Result<()> {
    let mut current = lock(&PARSE_CHECKPOINT_GATE);
    if current.is_some() {
        bail!("a parse-checkpoint gate is already installed");
    }
    *current = Some(gate);
    Ok(())
}

pub(super) fn pause_before_parse_checkpoint(path: &Path) {
    let gate = {
        let mut current = lock(&PARSE_CHECKPOINT_GATE);
        if current
            .as_ref()
            .is_some_and(|installed| installed.target_path.as_path() == path)
        {
            current.take()
        } else {
            None
        }
    };
    if let Some(gate) = gate {
        gate.pause();
    }
}

pub(in crate::session_search) fn clear_gate(expected: &Arc<ParseCheckpointGate>) {
    let mut current = lock(&PARSE_CHECKPOINT_GATE);
    if current
        .as_ref()
        .is_some_and(|installed| Arc::ptr_eq(installed, expected))
    {
        current.take();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn sanitized_error_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| {
            if cause.downcast_ref::<SessionOperationSettlement>().is_some() {
                "SessionOperationSettlement"
            } else if matches!(
                cause.downcast_ref::<sillage_ports::BoundedParseError>(),
                Some(sillage_ports::BoundedParseError::Budget(
                    sillage_ports::ParseBudgetFailure::Cancelled
                ))
            ) {
                "BoundedParseError::Budget(Cancelled)"
            } else if cause
                .downcast_ref::<sillage_ports::BoundedParseError>()
                .is_some()
            {
                "BoundedParseError(other)"
            } else {
                "other"
            }
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

#[tokio::test]
async fn public_prepare_classifies_cancellation_at_a_real_bounded_parse_checkpoint() -> Result<()> {
    let _serial = PARSE_TEST_SERIAL.lock().await;
    let (workspace, source) = Workspace::create()?;
    let checkpoint_path = source.clone();
    let owner = SessionPreparationOwner::new();
    let choice = SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source)?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })?;
    let (gate, reached_signal) = ParseCheckpointGate::new(checkpoint_path);
    install_gate(gate.clone())?;
    let cancellation = SessionSearchCancellation::new();
    let worker_owner = owner.clone();
    let worker_cancellation = cancellation.clone();
    let preparation = tokio::spawn(async move {
        worker_owner
            .prepare_cancellable(choice, worker_cancellation)
            .await
    });
    let reached = tokio::time::timeout(Duration::from_secs(10), reached_signal)
        .await
        .is_ok_and(|result| result.is_ok());
    cancellation.cancel();
    gate.release();
    let preparation = preparation
        .await
        .map_err(anyhow::Error::new)
        .context("public preparation caller failed to join")?;
    clear_gate(&gate);
    let classified = matches!(
        &preparation,
        Err(error)
            if error.downcast_ref::<SessionOperationSettlement>()
                == Some(&SessionOperationSettlement::RoutineCancellation)
    );
    let cooperative_parser_cause = matches!(
        &preparation,
        Err(error)
            if error.chain().any(|cause| {
                matches!(
                    cause.downcast_ref::<sillage_ports::BoundedParseError>(),
                    Some(sillage_ports::BoundedParseError::Budget(
                        sillage_ports::ParseBudgetFailure::Cancelled
                    ))
                )
            })
    );
    let diagnostic_chain = match &preparation {
        Err(error) => sanitized_error_chain(error),
        Ok(_) => "successful preparation".to_owned(),
    };
    let runtime_retirement = match &preparation {
        Ok(runtime) => runtime.retire().await,
        Err(_) => Ok(()),
    };
    let owner_retirement = owner.retire().await;
    drop(preparation);
    let cleanup = fs::remove_dir_all(&workspace.0)
        .context("cannot remove the public parser-cancellation workspace");
    runtime_retirement.context("unexpectedly prepared runtime could not retire")?;
    owner_retirement.context("preparation owner could not retire")?;
    cleanup?;
    if !reached || !classified || !cooperative_parser_cause {
        bail!(
            "bounded parse cancellation failed (selected-source checkpoint reached: {}; \
             typed cancellation settlement: {}; exact parser-cancel cause retained: {}; \
             sanitized error-chain types: [{}])",
            reached,
            classified,
            cooperative_parser_cause,
            diagnostic_chain
        );
    }
    Ok(())
}

#[path = "session_search_ingest_parse_deadline_tests.rs"]
mod deadline_tests;
