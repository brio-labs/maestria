use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};

use super::{
    PurgeTestAction, PurgeTestBoundary, arm_purge_test_action, clear_purge_test_action, hash_key,
};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);
const SOURCE_TEXT: &str = "# Notes\n\nThe saffron comet is an owned documentary generation.\n";
const SOURCE_MARKER_CONTENT: &str = "sillage-source-generations-v1\n";
const LATE_ENTRY_NAME: &str = "late-unknown.bin";
const LATE_ENTRY_CONTENT: &[u8] = b"preserve late unknown data";

struct Workspace(PathBuf);

impl Workspace {
    fn create(index: usize) -> Result<Self> {
        let parent = std::env::temp_dir();
        for _ in 0..128 {
            let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!(
                ".sillage-retained-fault-test-{}-{index}-{sequence}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("cannot create retained-store test root"),
            }
        }
        bail!("cannot allocate a retained-store fault workspace")
    }

    fn source(&self) -> PathBuf {
        self.0.join("selected.md")
    }

    fn store(&self) -> PathBuf {
        self.0.join("documentary-store")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn marker_and_directory_removal_failures_preserve_recoverable_owned_namespaces() -> Result<()>
{
    let cases = [
        (
            PurgeTestBoundary::GenerationMarkerUnlink,
            PurgeTestAction::Fail,
            true,
            false,
        ),
        (
            PurgeTestBoundary::GenerationDirectoryRemoval,
            PurgeTestAction::Fail,
            true,
            false,
        ),
        (
            PurgeTestBoundary::SourceMarkerUnlink,
            PurgeTestAction::Fail,
            false,
            false,
        ),
        (
            PurgeTestBoundary::SourceDirectoryRemoval,
            PurgeTestAction::Fail,
            false,
            false,
        ),
        (
            PurgeTestBoundary::GenerationDirectoryRemoval,
            PurgeTestAction::CreateUnknown {
                name: LATE_ENTRY_NAME,
                contents: LATE_ENTRY_CONTENT,
            },
            true,
            true,
        ),
        (
            PurgeTestBoundary::SourceDirectoryRemoval,
            PurgeTestAction::CreateUnknown {
                name: LATE_ENTRY_NAME,
                contents: LATE_ENTRY_CONTENT,
            },
            false,
            true,
        ),
    ];
    for (index, (boundary, action, has_generations, has_late_entry)) in
        cases.into_iter().enumerate()
    {
        run_fault_case(index, boundary, action, has_generations, has_late_entry)
            .await
            .with_context(|| format!("purge fault boundary {boundary:?}"))?;
    }
    Ok(())
}

async fn run_fault_case(
    index: usize,
    boundary: PurgeTestBoundary,
    action: PurgeTestAction,
    has_generations: bool,
    has_late_entry: bool,
) -> Result<()> {
    let workspace = Workspace::create(index)?;
    create_private_file(&workspace.source(), SOURCE_TEXT.as_bytes())?;
    let retention_root = workspace.store();
    let owner = crate::SessionPreparationOwner::with_retention_root(retention_root.clone())?;
    let runtime = owner
        .prepare_cancellable(
            selected_choice(&workspace.source())?,
            crate::SessionSearchCancellation::new(),
        )
        .await
        .context("cannot create an actual public retained-store generation")?;
    runtime
        .retire()
        .await
        .context("cannot retire fixture runtime")?;
    owner
        .retire()
        .await
        .context("cannot retire fixture owner")?;

    let source_key = hash_key(workspace.source().as_os_str().as_encoded_bytes());
    let source_directory = retention_root
        .join(".sillage-session-store-v1")
        .join("sources")
        .join(&source_key);
    let generations = source_directory.join("generations");
    let fault_directory = match boundary {
        PurgeTestBoundary::GenerationMarkerUnlink
        | PurgeTestBoundary::GenerationDirectoryRemoval => &generations,
        PurgeTestBoundary::SourceMarkerUnlink | PurgeTestBoundary::SourceDirectoryRemoval => {
            &source_directory
        }
    };
    let identity = arm_purge_test_action(boundary, fault_directory, action)?;
    let failure = owner.purge_retained_source(&workspace.source());
    let action_was_not_consumed = clear_purge_test_action(boundary, identity)?;
    if action_was_not_consumed {
        bail!("purge fault action did not match its exact directory identity and boundary");
    }
    if failure.is_ok() {
        bail!("injected purge transition unexpectedly succeeded");
    }
    let expected_source_marker = format!("sillage-selected-source-v1\n{source_key}\n");
    if fs::read(source_directory.join(".owner"))? != expected_source_marker.as_bytes() {
        bail!("source namespace ownership marker was not preserved or restored");
    }
    if has_generations && fs::read(generations.join(".owner"))? != SOURCE_MARKER_CONTENT.as_bytes()
    {
        bail!("generation ownership marker was not preserved or restored");
    }
    if has_late_entry {
        let entry = if boundary == PurgeTestBoundary::GenerationDirectoryRemoval {
            generations.join(LATE_ENTRY_NAME)
        } else {
            source_directory.join(LATE_ENTRY_NAME)
        };
        if fs::read(&entry)? != LATE_ENTRY_CONTENT {
            bail!("late unknown entry was not preserved after failed rmdir");
        }
        fs::remove_file(entry).context("cannot remove test-injected late unknown entry")?;
    }

    let retry_owner = crate::SessionPreparationOwner::with_retention_root(retention_root.clone())?;
    let prepared = retry_owner
        .prepare_cancellable(
            selected_choice(&workspace.source())?,
            crate::SessionSearchCancellation::new(),
        )
        .await
        .context("retained Prepare could not recover the verified owned namespace")?;
    prepared
        .retire()
        .await
        .context("recovered retained runtime could not retire")?;
    retry_owner
        .retire()
        .await
        .context("recovered retained owner could not retire")?;
    retry_owner
        .purge_retained_source(&workspace.source())
        .context("retry purge could not remove the recovered owned namespace")?;
    if count_generation_files(&retention_root)? != 0 {
        bail!("retry purge left documentary generations behind");
    }
    Ok(())
}

fn selected_choice(source: &Path) -> Result<SourceChoice> {
    Ok(SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source.to_path_buf())?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })?)
}

fn create_private_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("cannot create private test source {}", path.display()))?
        .write_all(contents)?;
    Ok(())
}

fn count_generation_files(root: &Path) -> Result<usize> {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut count = 0_usize;
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            count = count
                .checked_add(count_generation_files(&entry.path())?)
                .ok_or_else(|| anyhow!("retained generation count overflow"))?;
        } else if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("g-") && name.ends_with(".md"))
        {
            count = count
                .checked_add(1)
                .ok_or_else(|| anyhow!("retained generation count overflow"))?;
        }
    }
    Ok(count)
}
