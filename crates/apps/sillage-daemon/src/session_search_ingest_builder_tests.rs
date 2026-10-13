use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
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
use tokio_util::sync::CancellationToken;

use super::{SessionBuilder, SourceRead};
use crate::session_search::{
    deadline::SessionDeadline,
    ingest::{SessionStores, scan::NativeReadTestGate},
    native::{self, AcquiredFile},
    retained::{Admission, OwnershipPermit},
};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn create() -> Result<(Self, PathBuf)> {
        for _ in 0..128 {
            let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                ".sillage-builder-read-test-{}-{sequence}",
                std::process::id()
            ));
            let mut directory = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                directory.mode(0o700);
            }
            match directory.create(&root) {
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
                        .write_all(b"# Notes\n\nA real bounded read fixture.\n")?;
                    return Ok((Self(root), source));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        bail!("cannot allocate a private builder read workspace")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ReadObservation {
    reached: bool,
    deadline_elapsed: bool,
    deferred_entries: u32,
    result: Result<bool>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn elapsed_deadline_defers_only_typed_read_interruption() -> Result<()> {
    let (workspace, source) = Workspace::create()?;
    let choice = selected_choice(source.clone())?;
    let memory = Arc::new(sillage_search_tantivy::RamMemoryBudget::new(
        choice.budget().max_in_memory_bytes(),
    ));
    let stores = Arc::new(SessionStores::new(memory.clone())?);

    let write_only = OpenOptions::new().write(true).open(&source)?;
    let acquired = acquire_selected_file(&source, &memory)?.replace_file_for_test(write_only);
    let write_error =
        read_after_deadline(choice.clone(), source.clone(), stores.clone(), acquired).await?;
    let native_ebadf_preserved = write_error.result.as_ref().err().is_some_and(|error| {
        error.chain().any(|cause| {
            cause
                .downcast_ref::<io::Error>()
                .is_some_and(|cause| cause.raw_os_error() == Some(libc::EBADF))
        })
    });

    let readable = acquire_selected_file(&source, &memory)?;
    let typed_deadline = read_after_deadline(choice, source, stores, readable).await?;
    let typed_deadline_deferred = typed_deadline.result.as_ref().is_ok_and(|stopped| *stopped)
        && typed_deadline.deferred_entries == 1
        && typed_deadline.deadline_elapsed;
    if !write_error.reached
        || !write_error.deadline_elapsed
        || !native_ebadf_preserved
        || !typed_deadline.reached
        || !typed_deadline_deferred
    {
        bail!(
            "builder read interruption classification failed (real EBADF gate reached: {}; \
             deadline elapsed: {}; original EBADF preserved: {}; typed deadline gate reached: \
             {}; typed deadline intentionally deferred: {})",
            write_error.reached,
            write_error.deadline_elapsed,
            native_ebadf_preserved,
            typed_deadline.reached,
            typed_deadline_deferred
        );
    }
    drop(workspace);
    Ok(())
}

fn selected_choice(source: PathBuf) -> Result<SourceChoice> {
    SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source)?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::NoDocumentaryPersistence,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })
    .map_err(anyhow::Error::new)
}

fn acquire_selected_file(
    source: &Path,
    memory: &Arc<sillage_search_tantivy::RamMemoryBudget>,
) -> Result<AcquiredFile<OwnershipPermit>> {
    let cancellation = CancellationToken::new();
    let admission = Admission {
        memory,
        cancellation: &cancellation,
        deadline: SessionDeadline::after(Duration::from_secs(10)),
    };
    native::acquire_file(source, &admission).map_err(anyhow::Error::new)
}

async fn read_after_deadline(
    choice: SourceChoice,
    source: PathBuf,
    stores: Arc<SessionStores>,
    acquired: AcquiredFile<OwnershipPermit>,
) -> Result<ReadObservation> {
    let (gate, reached_signal) = NativeReadTestGate::new(acquired.file())?;
    NativeReadTestGate::install(gate.clone())?;
    let (deadline_sender, deadline_receiver) = tokio::sync::oneshot::channel();
    let worker = tokio::task::spawn_blocking(move || {
        let deadline = SessionDeadline::after(Duration::from_secs(2));
        let mut builder = SessionBuilder::new(&choice, stores, source.clone(), None, deadline);
        let _ = deadline_sender.send(deadline);
        let result = builder
            .read_source(acquired, &CancellationToken::new())
            .map(|read| matches!(read, SourceRead::Stop));
        (result, builder.coverage.deferred_entries)
    });
    let deadline = deadline_receiver.await;
    let reached = tokio::time::timeout(Duration::from_secs(10), reached_signal)
        .await
        .is_ok_and(|signal| signal.is_ok());
    if reached && let Ok(deadline) = &deadline {
        tokio::time::sleep(
            deadline
                .remaining()
                .saturating_add(Duration::from_millis(20)),
        )
        .await;
    }
    gate.release();
    let joined = worker
        .await
        .map_err(anyhow::Error::new)
        .context("native read worker failed to join");
    NativeReadTestGate::clear(&gate);
    let (result, deferred_entries) = joined?;
    Ok(ReadObservation {
        reached,
        deadline_elapsed: deadline
            .as_ref()
            .is_ok_and(|deadline| deadline.is_elapsed()),
        deferred_entries,
        result,
    })
}
