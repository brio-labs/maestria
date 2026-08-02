use anyhow::{Context, Result};
use maestria_core::InstanceLayout;
use maestria_core::InstanceManifest;
use maestria_daemon::db_retry::{is_database_busy, run_database_retry};
use maestria_daemon::ingestion_policy::{is_privacy_excluded_path, is_supported_source_file};
use maestria_domain::KernelState;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::{sleep, timeout};

pub(crate) fn ensure_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    maestria_daemon::prepare_instance(instance_dir)
}

pub(crate) fn validated_instance(instance_dir: PathBuf) -> Result<InstanceLayout> {
    let layout = InstanceLayout::for_root(instance_dir);
    if !layout.manifest_path.exists() {
        return Err(anyhow::anyhow!(
            "instance manifest is missing at {}; run init first",
            layout.manifest_path.display()
        ));
    }
    load_manifest(&layout)?;
    Ok(layout)
}

pub(crate) fn load_manifest(layout: &InstanceLayout) -> Result<InstanceManifest> {
    let contents = fs::read_to_string(&layout.manifest_path)
        .with_context(|| format!("read instance manifest {}", layout.manifest_path.display()))?;
    maestria_core::InstanceService::parse_manifest(&contents)
        .map_err(|error| anyhow::anyhow!("parse instance manifest: {error}"))
}

pub(crate) fn load_kernel_state_with_retry(
    layout: &InstanceLayout,
    context: &'static str,
) -> Result<KernelState> {
    retry_db_busy(context, || {
        maestria_daemon::load_kernel_state(layout).with_context(|| context)
    })
}

/// Retry a synchronous database operation while the instance is transiently
/// locked, delegating the retry cadence to the shared daemon policy
/// (`maestria_daemon::db_retry`).
///
/// The shared daemon constants (`RETRY_ATTEMPTS` / `RETRY_DELAY`) drive the
/// loop. A busy error that outlives the retry budget is reported as a
/// timeout with the last underlying error, matching the historical CLI
/// wording.
pub(crate) fn retry_db_busy<T>(context: &str, operation: impl Fn() -> Result<T>) -> Result<T> {
    match run_database_retry(operation) {
        Ok(output) => Ok(output),
        Err(error) if is_database_busy(&error) => {
            Err(anyhow::anyhow!("timed out while {context}: {error}"))
        }
        Err(error) => Err(error),
    }
}

/// Poll persisted kernel state until `predicate` holds, within `timeout_budget`.
///
/// This is the single CLI policy for waiting on durable kernel state: command
/// modules pass a predicate instead of restating the poll-and-retry loop.
/// Transient database-lock errors are detected with the shared daemon matcher
/// ([`is_db_locked`]) and retried at the CLI polling cadence; the last such
/// error is preserved in the timeout message. The returned state is the one
/// that satisfied the predicate.
pub(crate) async fn wait_for_kernel_state(
    layout: &InstanceLayout,
    timeout_budget: Duration,
    wait_context: String,
    predicate: impl Fn(&KernelState) -> bool,
) -> Result<KernelState> {
    let last_error = Arc::new(Mutex::new(None::<String>));
    let last_error_for_wait = Arc::clone(&last_error);
    let result = timeout(timeout_budget, async {
        loop {
            match maestria_daemon::load_kernel_state(layout)
                .with_context(|| format!("load kernel state while {wait_context}"))
            {
                Ok(state) => {
                    if predicate(&state) {
                        return Ok::<_, anyhow::Error>(state);
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) if is_database_busy(&error) => {
                    if let Ok(mut slot) = last_error_for_wait.lock() {
                        *slot = Some(error.to_string());
                    }
                    sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        }
    })
    .await;

    match result {
        Ok(Ok(state)) => Ok(state),
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => {
            let detail = last_error
                .lock()
                .ok()
                .and_then(|error| error.clone())
                .map_or_else(String::new, |error| format!(" {error}"));
            Err(anyhow::anyhow!("timed out while {wait_context}{detail}"))
        }
    }
}

pub(crate) fn collect_index_files(path: &Path, recursive: bool) -> Result<Vec<PathBuf>> {
    if is_excluded_index_path(path) {
        return Err(anyhow::anyhow!(
            "index path is excluded by privacy policy: {}",
            path.display()
        ));
    }
    if is_symlink(path)? {
        return Err(anyhow::anyhow!(
            "index path is a symlink and is not indexed: {}",
            path.display()
        ));
    }
    if path.is_file() {
        if !is_supported_index_path(path) {
            return Err(anyhow::anyhow!(
                "unsupported index file type: {}",
                path.display()
            ));
        }
        return Ok(vec![path.to_path_buf()]);
    }
    if !path.is_dir() {
        return Err(anyhow::anyhow!(
            "index path does not exist: {}",
            path.display()
        ));
    }
    if !recursive {
        return Err(anyhow::anyhow!(
            "{} is a directory; pass --recursive to index contained files",
            path.display()
        ));
    }

    let mut files = Vec::new();
    let walker = ignore::WalkBuilder::new(path)
        .hidden(true)
        .ignore(true)
        .git_ignore(true)
        .require_git(false)
        .follow_links(false)
        .build();

    for result in walker {
        let entry = result?;
        let entry_path = entry.path();
        if let Some(error) = entry.error() {
            return Err(anyhow::anyhow!(
                "index traversal failed at {}: {error}",
                entry_path.display()
            ));
        }

        if entry
            .file_type()
            .is_some_and(|file_type| file_type.is_symlink())
        {
            continue;
        }
        if is_excluded_index_path(entry_path) {
            continue;
        }

        if entry_path.is_file() && is_supported_index_path(entry_path) {
            files.push(entry_path.to_path_buf());
        }
    }

    files.sort();
    Ok(files)
}

pub(crate) fn is_excluded_index_path(path: &Path) -> bool {
    is_privacy_excluded_path(path)
}

fn is_symlink(path: &Path) -> Result<bool> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(false)
}

pub(crate) fn is_supported_index_path(path: &Path) -> bool {
    is_supported_source_file(path)
}

pub(crate) fn source_label(evidence: &maestria_domain::Evidence) -> String {
    use maestria_domain::EvidenceKind;

    match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            snapshot,
        } => format!(
            "source=file path={} lines={}-{} hash={}",
            path,
            range.start(),
            range.end(),
            snapshot.content_hash().as_str()
        ),
        EvidenceKind::PdfSpan {
            snapshot,
            page_start,
            page_end,
        } => format!(
            "source=pdf blob={} pages={}-{} hash={}",
            snapshot.blob_id(),
            page_start,
            page_end,
            snapshot.content_hash().as_str()
        ),
        EvidenceKind::PdfRegion {
            snapshot,
            page,
            x,
            y,
            width,
            height,
        } => format!(
            "source=pdf blob={} page={} region={},{} {}x{} hash={}",
            snapshot.blob_id(),
            page,
            x,
            y,
            width,
            height,
            snapshot.content_hash().as_str()
        ),
        EvidenceKind::WebSnapshot { url, snapshot, .. } => {
            format!("source=web url={} snapshot={}", url, snapshot.blob_id())
        }
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            blob,
        } => format!(
            "source=command run={} stream={:?} blob={}",
            harness_run, stream, blob
        ),
        EvidenceKind::TestResult {
            harness_run,
            status,
            log,
        } => format!(
            "source=test run={} status={:?} log={}",
            harness_run, status, log
        ),
        EvidenceKind::Diff {
            harness_run,
            patch_blob,
        } => format!("source=diff run={} patch={}", harness_run, patch_blob),
        EvidenceKind::Validation { report_id } => {
            format!("source=validation report={}", report_id)
        }
    }
}
