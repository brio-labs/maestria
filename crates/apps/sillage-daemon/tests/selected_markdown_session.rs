use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result, anyhow, bail};
use sillage_daemon::{SessionPreparationOwner, SessionSearchCancellation, StaleEvidence};
use sillage_domain::{
    ComputationTiming, ControlPersistenceChoice, DocumentaryRetentionChoice, SourceAvailability,
    SourceChoice, SourceChoiceParameters, SourceLocality, SourceRepresentation,
    SourceResourceBudget, SourceScope,
};

static NEXT_WORKSPACE: AtomicU64 = AtomicU64::new(0);
const SOURCE: &str = "# Field notes\n\nThe saffron comet crosses the observatory at midnight.\n";
const CHANGED_SOURCE: &str = "# Field notes\n\nThe notebook changed after citation.\n";

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Result<Self> {
        let parent = std::env::temp_dir();
        for _ in 0..128 {
            let sequence = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
            let root = parent.join(format!(
                ".sillage-selected-session-test-{}-{sequence}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&root) {
                Ok(()) => return Ok(Self(root)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("cannot create private test workspace"),
            }
        }
        bail!("cannot allocate an exclusive private test workspace")
    }

    fn source(&self) -> PathBuf {
        self.0.join("selected.md")
    }

    fn retention_root(&self) -> PathBuf {
        self.0.join("retention")
    }

    fn write_source(&self, contents: &str) -> Result<()> {
        fs::write(self.source(), contents).context("cannot update selected test Markdown")
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn choice(source: PathBuf, retention: DocumentaryRetentionChoice) -> Result<SourceChoice> {
    Ok(SourceChoice::try_new(SourceChoiceParameters {
        scope: SourceScope::file(source)?,
        timing: ComputationTiming::OnDemand,
        locality: SourceLocality::LocalOnly,
        documentary_retention: retention,
        control_persistence: ControlPersistenceChoice::SessionOnly,
        availability: SourceAvailability::Enabled,
        representations: vec![SourceRepresentation::LexicalIndex],
        remote_disclosures: Vec::new(),
        budget: SourceResourceBudget::session_default(),
    })?)
}

fn write_private_marker_or_source(path: &Path, contents: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("cannot create private test file {}", path.display()))?
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
        if entry.file_type()?.is_dir() {
            count = count
                .checked_add(count_generation_files(&entry.path())?)
                .ok_or_else(|| anyhow!("retained generation count overflow"))?;
        } else if entry.file_type()?.is_file()
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

#[tokio::test]
async fn repeated_retention_listing_then_retire_and_purge_removes_generation() -> Result<()> {
    let workspace = Workspace::new()?;
    write_private_marker_or_source(&workspace.source(), SOURCE.as_bytes())?;
    let owner = SessionPreparationOwner::with_retention_root(workspace.retention_root())?;
    let runtime = owner
        .prepare_cancellable(
            choice(
                workspace.source(),
                DocumentaryRetentionChoice::RetainUntilExplicitlyPurged,
            )?,
            SessionSearchCancellation::new(),
        )
        .await?;
    let result = runtime
        .search("saffron comet".to_string(), 8)
        .await
        .context("selected Markdown search failed")?;
    let citation = result
        .previews
        .first()
        .ok_or_else(|| anyhow!("selected Markdown citation was not produced"))?;
    let reopened = runtime
        .preview_evidence(
            citation.evidence_id,
            citation.artifact_version,
            citation.source_revision,
            SessionSearchCancellation::new(),
        )
        .await?;
    if reopened.content_hash != citation.content_hash || reopened.excerpt != citation.excerpt {
        bail!("fresh public preview changed its selected-source citation");
    }
    let retention_root = workspace.retention_root();
    if count_generation_files(&retention_root)? != 1
        || count_generation_files(&retention_root)? != 1
    {
        bail!("repeated retention listing did not preserve the owned generation");
    }
    workspace.write_source(CHANGED_SOURCE)?;
    let stale = runtime
        .preview_evidence(
            citation.evidence_id,
            citation.artifact_version,
            citation.source_revision,
            SessionSearchCancellation::new(),
        )
        .await;
    match stale {
        Err(error) if error.downcast_ref::<StaleEvidence>().is_some() => {}
        Err(error) => return Err(error.context("mutation refusal was not typed as stale evidence")),
        Ok(_) => bail!("changed source produced an authorized stale preview"),
    }
    runtime
        .retire()
        .await
        .context("runtime retirement failed")?;
    owner.retire().await.context("owner retirement failed")?;
    if count_generation_files(&retention_root)? != 1 {
        bail!("retirement implicitly removed retained documentary data");
    }
    owner.purge_retained_source(&workspace.source())?;
    if count_generation_files(&retention_root)? != 0 {
        bail!("explicit purge left retained source generations behind");
    }
    Ok(())
}

#[tokio::test]
async fn retired_runtimes_release_native_sources_while_runtime_values_are_held() -> Result<()> {
    const HANDLE_LIMIT: usize = 64;
    let workspace = Workspace::new()?;
    write_private_marker_or_source(&workspace.source(), SOURCE.as_bytes())?;
    let mut retired_runtimes = Vec::new();
    retired_runtimes.try_reserve_exact(HANDLE_LIMIT + 1)?;
    for index in 0..=HANDLE_LIMIT {
        let owner = SessionPreparationOwner::new();
        let runtime = owner
            .prepare_cancellable(
                choice(
                    workspace.source(),
                    DocumentaryRetentionChoice::NoDocumentaryPersistence,
                )?,
                SessionSearchCancellation::new(),
            )
            .await
            .with_context(|| format!("cannot prepare source after {index} retired runtimes"))?;
        runtime.retire().await?;
        owner.retire().await?;
        let retired_search = runtime.search("saffron comet".to_string(), 1).await;
        match retired_search {
            Err(error) if error.to_string().contains("retired") => {}
            Err(error) => return Err(error.context("retired runtime failed for another reason")),
            Ok(_) => bail!("retired runtime still admitted a search"),
        }
        retired_runtimes.push(runtime);
    }
    if retired_runtimes.len() != HANDLE_LIMIT + 1 {
        bail!("retired runtime values were not held through source re-admission");
    }
    Ok(())
}
