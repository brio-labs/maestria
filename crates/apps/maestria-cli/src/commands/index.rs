use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest, artifact_id_for, content_hash};
use maestria_domain::{
    ArtifactDetected, ArtifactId, DomainInput, IndexStatus, KernelState, TaskId,
};
use maestria_governance::{PrivacyExclusions, Scope};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::helpers;

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Shared context for processing a single file through indexing.
struct ProcessContext<'a> {
    scope: &'a Scope,
    privacy: &'a PrivacyExclusions,
    manifest: &'a InstanceManifest,
    preexisting_state: &'a KernelState,
    session: &'a maestria_daemon::MutationSession,
    layout: &'a InstanceLayout,
    index_timeout: Duration,
}

/// Process a single file through read, scope check, duplicate detection,
/// runtime submission, and wait-for-indexing.
///
/// Returns an error for scope violations, I/O failures, runtime submission errors,
/// Terminal per-file outcome for the batch summary.
enum FileOutcome {
    Indexed,
    Unchanged,
    Skipped(String),
}

/// Count one policy skip under its reason, preserving insertion order.
fn record_skip(skipped: &mut Vec<(&'static str, usize)>, reason: &'static str) {
    if let Some((_, count)) = skipped.iter_mut().find(|(name, _)| *name == reason) {
        *count += 1;
    } else {
        skipped.push((reason, 1));
    }
}

/// Ask the user how to treat the subtree `dir`, bounded so a batch never
/// becomes a questionnaire:
/// - only notable children (many files or large total size) are asked
///   about; everything else inherits the parent decision;
/// - `l` drills one level deeper (up to [`MAX_PROMPT_DEPTH`]);
/// - at most [`MAX_PROMPT_CHILDREN`] children are listed per level;
/// - the answers are recorded as exclusions: `n` disables exactly that
///   subtree, everything else stays indexed.
fn prompt_directory(
    dir: &Path,
    files: &[PathBuf],
    depth: usize,
    approval: &mut super::index_policy::Approval,
    accept_all: &mut bool,
) -> Result<()> {
    let groups = super::index_policy::group_by_child(dir, files);
    let mut prompted = 0usize;
    for (child, count, total_bytes) in groups {
        if *accept_all || !super::index_policy::is_notable_group(count, total_bytes) {
            continue;
        }
        prompted += 1;
        if prompted > MAX_PROMPT_CHILDREN {
            continue;
        }
        let size_mb = total_bytes as f64 / (1024.0 * 1024.0);
        loop {
            let options = if depth < MAX_PROMPT_DEPTH {
                "[Y/n/l/a/q]"
            } else {
                "[Y/n/a/q]"
            };
            println!(
                "Index everything under {}? ({count} files, {size_mb:.1} MB) {options}",
                child.display()
            );
            print!("> ");
            let _ = std::io::Write::flush(&mut std::io::stdout());
            let mut answer = String::new();
            if std::io::stdin().read_line(&mut answer).is_err() {
                return Err(anyhow!("failed to read approval answer"));
            }
            match answer.trim().to_ascii_lowercase().as_str() {
                "" | "y" | "yes" => break,
                "n" | "no" => {
                    approval.add_skip(child.clone());
                    break;
                }
                "l" | "list" if depth < MAX_PROMPT_DEPTH => {
                    let child_files: Vec<PathBuf> = files
                        .iter()
                        .filter(|file| file.starts_with(&child))
                        .cloned()
                        .collect();
                    prompt_directory(&child, &child_files, depth + 1, approval, accept_all)?;
                    break;
                }
                "l" | "list" => {
                    println!("This directory is already at the deepest drill-down level.");
                }
                "a" | "all" => {
                    *accept_all = true;
                    break;
                }
                "q" | "quit" => return Err(anyhow!("aborted by user")),
                _ => {}
            }
        }
    }
    Ok(())
}

/// Deepest drill-down level (root group = 1, so level 4 reaches folders
/// inside a repository).
const MAX_PROMPT_DEPTH: usize = 4;
const MAX_PROMPT_CHILDREN: usize = 6;

/// Offer the collected sources for approval with bounded drill-down.
fn approve_groups_interactively(
    root: &Path,
    files: &[PathBuf],
) -> Result<super::index_policy::Approval> {
    let mut approval = super::index_policy::Approval::all();
    let mut accept_all = false;
    prompt_directory(root, files, 1, &mut approval, &mut accept_all)?;
    Ok(approval)
}

/// Process one file, or return the error that prevented it from reaching a
/// terminal state. The caller continues the batch on error; the summary
/// reports the failure.
async fn process_file(file: &Path, ctx: &ProcessContext<'_>) -> Result<FileOutcome> {
    let file = file
        .canonicalize()
        .with_context(|| format!("canonicalize index path {}", file.display()))?;
    // Preserve scope, privacy, and manifest checks before reading.
    if ctx.scope.check_read_containment(&file).is_err()
        || ctx.privacy.is_excluded(&file)
        || !ctx.manifest.allows_source(&file)
    {
        return Err(anyhow!(
            "index path is outside the instance read scope or excluded by policy: {}",
            file.display()
        ));
    }

    let bytes = fs::read(&file)?;
    let artifact_id = artifact_id_for(&file, &bytes);
    let hash = maestria_domain::ContentHash::new(content_hash(&bytes))?;
    // Check whether this exact artifact was already indexed before this session.
    if let Some(artifact) = ctx.preexisting_state.artifacts.get(&artifact_id)
        && artifact.content_hash.as_ref() == Some(&hash)
        && artifact.index_status == IndexStatus::Indexed
    {
        println!("unchanged artifact={} path={}", artifact.id, file.display());
        return Ok(FileOutcome::Unchanged);
    }

    let title = match file.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.to_string(),
        None => "unknown".to_string(),
    };

    ctx.session
        .submit(DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id,
            title,
            source_path: file.display().to_string(),
            source_bytes: bytes,
            content_hash: hash,
        }))
        .await
        .context("failed to submit artifact to runtime")?;

    // Wait for the artifact to reach terminal persisted state. Unsupported,
    // failed, and quarantined parses carry no indexable content, so the
    // artifact never becomes `Indexed`; a terminal non-`Parsed` parse status
    // is done and reported as skipped. The predicate records the outcome.
    // Polls the artifact row directly — replaying the full event log per
    // poll would dominate batch time as the log grows.
    let outcome = std::cell::RefCell::new(None);
    helpers::wait_for_artifact_state(
        ctx.layout,
        artifact_id,
        ctx.index_timeout,
        format!("waiting for artifact indexing: {}", file.display()),
        |artifact| {
            if artifact.index_status == IndexStatus::Indexed {
                outcome.replace(Some(FileOutcome::Indexed));
                return true;
            }
            let skipped = match artifact.parse_status {
                Some(maestria_domain::ParseStatus::Unsupported) => Some("unsupported"),
                Some(maestria_domain::ParseStatus::Failed) => Some("failed"),
                Some(maestria_domain::ParseStatus::Quarantined) => Some("quarantined"),
                _ => None,
            };
            if let Some(reason) = skipped {
                outcome.replace(Some(FileOutcome::Skipped(reason.to_string())));
                true
            } else {
                false
            }
        },
    )
    .await?;
    let outcome = outcome
        .into_inner()
        .ok_or_else(|| anyhow!("artifact {artifact_id} has no terminal outcome"))?;
    match &outcome {
        FileOutcome::Indexed => {
            println!("indexed artifact={artifact_id} path={}", file.display());
        }
        FileOutcome::Unchanged => {}
        FileOutcome::Skipped(reason) => {
            println!(
                "skipped artifact={artifact_id} path={} ({reason})",
                file.display()
            );
        }
    }
    Ok(outcome)
}

/// Wait until every artifact in `recovery_artifact_ids` has reached
/// `IndexStatus::Indexed`, or until `recovery_timeout` elapses.
async fn drain_recovery(
    layout: &InstanceLayout,
    recovery_artifact_ids: &[ArtifactId],
    recovery_timeout: Duration,
) -> Result<()> {
    helpers::wait_for_kernel_state(
        layout,
        recovery_timeout,
        "waiting for recovery artifact indexing".to_string(),
        |state| {
            recovery_artifact_ids.iter().all(|id| {
                state
                    .artifacts
                    .get(id)
                    .is_some_and(|a| a.index_status == IndexStatus::Indexed)
            })
        },
    )
    .await?;
    Ok(())
}

/// Wait until every recovered validation task has a durable validation
/// report, or until `recovery_timeout` elapses.
async fn drain_validation_recovery(
    layout: &InstanceLayout,
    validation_task_ids: &[TaskId],
    recovery_timeout: Duration,
) -> Result<()> {
    helpers::wait_for_kernel_state(
        layout,
        recovery_timeout,
        "waiting for recovered task validation reports".to_string(),
        |state| {
            validation_task_ids
                .iter()
                .all(|task_id| maestria_daemon::has_current_validation_report(state, *task_id))
        },
    )
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public command
// ---------------------------------------------------------------------------

/// Index files into the instance under the mutation session.
///
/// # Cancellation
/// Dropping this future tears down the CLI-side session (instance lock
/// released, runtime shutdown requested). Files already accepted by the
/// runtime may still be indexed to durable state; inspect the index before
/// retrying an interrupted command.
pub async fn run(
    instance_dir: PathBuf,
    path: PathBuf,
    recursive: bool,
    mode: super::index_policy::IndexMode,
    yes: bool,
) -> Result<()> {
    let layout = helpers::ensure_instance(instance_dir)?;
    let manifest = helpers::load_manifest(&layout)?;
    let scope = Scope::new(
        manifest.read_roots.clone(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        false,
    );
    let privacy = PrivacyExclusions::default();
    let files = helpers::collect_index_files(&path, recursive)?;
    if files.is_empty() {
        return Err(anyhow!(
            "no files selected for indexing at {}",
            path.display()
        ));
    }

    // Approval pass: in `Lazy` and `Smart` modes, notable top-level groups
    // (many files or large total size) are offered to the user for approval
    // before anything is submitted. `Simple` never prompts — it indexes
    // everything, by contract. Scripted runs (no terminal) and `--yes`
    // approve everything; the mode policy then decides what is skipped.
    let approval = if mode == super::index_policy::IndexMode::Simple || yes {
        super::index_policy::Approval::all()
    } else if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        approve_groups_interactively(&path, &files)?
    } else {
        super::index_policy::Approval::all()
    };

    // Policy pass: the chosen mode decides which collected sources are
    // indexed. Skipped sources are counted by reason and never submitted,
    // so `Smart` and `Lazy` avoid paying parse and index cost on content
    // the user opted out of.
    let mut policy_skipped: Vec<(&'static str, usize)> = Vec::new();
    let mut selected_files = Vec::with_capacity(files.len());
    for file in &files {
        if !approval.allows(file) {
            record_skip(&mut policy_skipped, "unapproved");
            continue;
        }
        if !manifest.allows_source(file) || privacy.is_excluded(file) {
            record_skip(&mut policy_skipped, "excluded");
            continue;
        }
        let size = fs::metadata(file).map_or(0, |metadata| metadata.len());
        match super::index_policy::select_source(file, size, mode) {
            super::index_policy::Selection::Index => selected_files.push(file.clone()),
            super::index_policy::Selection::Skip(reason) => {
                record_skip(&mut policy_skipped, reason);
            }
        }
    }
    for (reason, count) in &policy_skipped {
        println!("policy skipped {count} sources ({reason})");
    }

    let session = maestria_daemon::MutationSession::start(
        layout.clone(),
        maestria_governance::AutonomyProfile::TrustedWorkspace,
    )
    .await?;
    if session.paused_effect_count() > 0 {
        println!(
            "paused {} in-flight harness effects",
            session.paused_effect_count()
        );
    }
    let preexisting_state = session.state().clone();

    let result = async {
        let recovery = session.recovery().clone();
        let index_timeout = Duration::from_secs(30);
        let ctx = ProcessContext {
            scope: &scope,
            privacy: &privacy,
            manifest: &manifest,
            preexisting_state: &preexisting_state,
            session: &session,
            layout: &layout,
            index_timeout,
        };

        // Per-file supervision: one unindexable or failing file must not
        // abort the batch. Outcomes are counted; failures are reported in
        // the summary and reflected in the exit status.
        let mut indexed = 0usize;
        let mut unchanged = 0usize;
        let mut skipped = 0usize;
        let mut failed = 0usize;
        for file in &selected_files {
            match process_file(file, &ctx).await {
                Ok(FileOutcome::Indexed) => indexed += 1,
                Ok(FileOutcome::Unchanged) => unchanged += 1,
                Ok(FileOutcome::Skipped(_)) => skipped += 1,
                Err(error) => {
                    failed += 1;
                    eprintln!("failed artifact path={} error={error}", file.display());
                }
            }
        }

        if !recovery.artifact_ids.is_empty() {
            drain_recovery(&layout, &recovery.artifact_ids, Duration::from_secs(60)).await?;
        }
        if !recovery.validation_task_ids.is_empty() {
            drain_validation_recovery(
                &layout,
                &recovery.validation_task_ids,
                Duration::from_secs(60),
            )
            .await?;
        }
        println!("indexed {indexed} · unchanged {unchanged} · skipped {skipped} · failed {failed}");
        if failed > 0 {
            Err(anyhow!(
                "{failed} of {} selected files failed to index",
                selected_files.len()
            ))
        } else {
            Ok(())
        }
    }
    .await;

    session.finish(result).await
}
