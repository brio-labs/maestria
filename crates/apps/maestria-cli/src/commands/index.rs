use anyhow::{Context, Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest, artifact_id_for, content_hash};
use maestria_domain::{
    ArtifactDetected, ArtifactId, DomainInput, IndexStatus, KernelState, TaskId,
};
use maestria_governance::{PrivacyExclusions, Scope};
use maestria_index_selection::{IndexPolicy, Selection};
use std::{
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

use super::index_metrics::{IndexMetrics, human_bytes};
use super::index_selection::{SelectionPlan, approve_interactively, approve_scripted, record_skip};
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

/// Process one file, or return the error that prevented it from reaching a
/// terminal state. The caller continues the batch on error; the summary
/// reports the failure. Returns the number of source bytes read alongside
/// the terminal outcome for aggregate throughput metrics.
async fn process_file(
    file: &Path,
    done: usize,
    total: usize,
    ctx: &ProcessContext<'_>,
) -> Result<(FileOutcome, u64)> {
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
    let bytes_len = bytes.len() as u64;
    let prefix = format!("[{done}/{total}]");
    let size = human_bytes(bytes_len);
    let artifact_id = artifact_id_for(&file, &bytes);
    let hash = maestria_domain::ContentHash::new(content_hash(&bytes))?;
    // Check whether this exact artifact was already indexed before this session.
    if let Some(artifact) = ctx.preexisting_state.artifacts.get(&artifact_id)
        && artifact.content_hash.as_ref() == Some(&hash)
        && artifact.index_status == IndexStatus::Indexed
    {
        println!(
            "{prefix} unchanged artifact={} path={} ({size})",
            artifact.id,
            file.display()
        );
        return Ok((FileOutcome::Unchanged, bytes_len));
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
            println!(
                "{prefix} indexed artifact={artifact_id} path={} ({size})",
                file.display()
            );
        }
        FileOutcome::Unchanged => {}
        FileOutcome::Skipped(reason) => {
            println!(
                "{prefix} skipped artifact={artifact_id} path={} ({size}, {reason})",
                file.display()
            );
        }
    }
    Ok((outcome, bytes_len))
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

/// Index every selected file under per-file supervision, wait for recovery
/// work, and print the run summary with live metrics. Returns `Ok(())` when
/// no selected file failed.
async fn run_selected_batch(
    selected_files: &[PathBuf],
    ctx: &ProcessContext<'_>,
    recovery: &maestria_daemon::RecoveryQueue,
    policy_skipped_total: usize,
) -> Result<()> {
    let mut metrics = IndexMetrics::new(selected_files.len(), ctx.layout);
    let mut indexed = 0usize;
    let mut unchanged = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for (done, file) in selected_files.iter().enumerate() {
        let done = done + 1;
        match process_file(file, done, selected_files.len(), ctx).await {
            Ok((FileOutcome::Indexed, bytes)) => {
                indexed += 1;
                metrics.add_bytes(bytes);
            }
            Ok((FileOutcome::Unchanged, bytes)) => {
                unchanged += 1;
                metrics.add_bytes(bytes);
            }
            Ok((FileOutcome::Skipped(_), bytes)) => {
                skipped += 1;
                metrics.add_bytes(bytes);
            }
            Err(error) => {
                failed += 1;
                eprintln!("failed artifact path={} error={error}", file.display());
            }
        }
        if let Some(line) = metrics.status_line(done) {
            println!("{line}");
        }
    }

    if !recovery.artifact_ids.is_empty() {
        drain_recovery(ctx.layout, &recovery.artifact_ids, Duration::from_secs(60)).await?;
    }
    if !recovery.validation_task_ids.is_empty() {
        drain_validation_recovery(
            ctx.layout,
            &recovery.validation_task_ids,
            Duration::from_secs(60),
        )
        .await?;
    }
    println!(
        "{}",
        metrics.summary(indexed, unchanged, skipped + policy_skipped_total, failed)
    );
    if failed > 0 {
        Err(anyhow!(
            "{failed} of {} selected files failed to index",
            selected_files.len()
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public command
// ---------------------------------------------------------------------------

/// Index files into the instance under the mutation session.
///
/// A single file is indexed directly under the batch policy (or no
/// filtering). A directory is first classified by the choice layer; the
/// user approves a whitelist interactively (or it is built automatically
/// on a non-TTY run / `--yes`), and only whitelisted files are submitted.
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
    batch_policy: Option<IndexPolicy>,
    yes: bool,
    save_selection: bool,
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

    let plan = plan_selection(&path, recursive, batch_policy, yes, save_selection, &layout)?;

    let files = maestria_index_selection::collect_files(&path, recursive)?;
    if files.is_empty() {
        return Err(anyhow!(
            "no files selected for indexing at {}",
            path.display()
        ));
    }

    // Selection pass: the whitelist decides which collected sources are
    // indexed; the per-file policy is the deepest approved ancestor's
    // override (or the batch policy). Skipped sources are counted by
    // reason and never submitted, so filtered directories avoid paying
    // parse and index cost on content the user opted out of.
    let (selected_files, policy_skipped) =
        select_batch(&files, &plan, batch_policy, &manifest, &privacy);
    for (reason, count) in &policy_skipped {
        println!("policy skipped {count} sources ({reason})");
    }
    let policy_skipped_total: usize = policy_skipped.iter().map(|(_, count)| count).sum();

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
        run_selected_batch(&selected_files, &ctx, &recovery, policy_skipped_total).await
    }
    .await;

    session.finish(result).await
}

/// Build the whitelist plan for `path`.
///
/// A direct file target is approved under the batch policy (all switches
/// off when no flags are given) with no classification or prompts. A
/// directory is classified by the choice layer, then the user approves
/// the candidates interactively (or the plan is built automatically on a
/// non-TTY run / `--yes`); `--save-selection` persists the result.
fn plan_selection(
    path: &Path,
    recursive: bool,
    batch_policy: Option<IndexPolicy>,
    yes: bool,
    save_selection: bool,
    layout: &InstanceLayout,
) -> Result<SelectionPlan> {
    if path.is_file() {
        let policy = if let Some(policy) = batch_policy {
            policy
        } else {
            IndexPolicy::everything()
        };
        let mut plan = SelectionPlan::default();
        plan.approve_path(path, policy);
        return Ok(plan);
    }
    if !recursive {
        return Err(anyhow!(
            "{} is a directory; pass --recursive to index contained files",
            path.display()
        ));
    }
    // Whitelist-first selection: classify the tree, then let the user
    // approve (or auto-approve) the candidate directories. The root node
    // itself is a container, not a candidate — the walk starts at the
    // top-level groups so the whitelist can exclude subtrees.
    let tree = maestria_index_selection::scan_candidates(path)?;
    let plan = if IsTerminal::is_terminal(&std::io::stdin()) && !yes {
        approve_interactively(&tree, batch_policy)?
    } else {
        let mut plan = SelectionPlan::default();
        for child in &tree.children {
            approve_scripted(child, batch_policy, yes, &mut plan);
        }
        plan
    };
    if save_selection {
        let profile = maestria_index_selection::IndexSelectionProfile {
            root: path.to_path_buf(),
            includes: plan.includes().to_vec(),
            policies: plan.policies().clone(),
        };
        maestria_index_selection::save_profile(
            &layout.system_dir.join("index-selection.json"),
            &profile,
        )?;
    }
    Ok(plan)
}

/// Apply the whitelist and the per-file policies to the collected files.
fn select_batch(
    files: &[PathBuf],
    plan: &SelectionPlan,
    batch_policy: Option<IndexPolicy>,
    manifest: &InstanceManifest,
    privacy: &PrivacyExclusions,
) -> (Vec<PathBuf>, Vec<(&'static str, usize)>) {
    let mut policy_skipped: Vec<(&'static str, usize)> = Vec::new();
    let mut selected_files = Vec::with_capacity(files.len());
    for file in files {
        if !plan.allows(file) {
            record_skip(&mut policy_skipped, "unapproved");
            continue;
        }
        if !manifest.allows_source(file) || privacy.is_excluded(file) {
            record_skip(&mut policy_skipped, "excluded");
            continue;
        }
        let size = fs::metadata(file).map_or(0, |metadata| metadata.len());
        let policy = plan.file_policy(file, batch_policy);
        match maestria_index_selection::select_source(file, size, policy) {
            Selection::Index => selected_files.push(file.clone()),
            Selection::Skip(reason) => {
                record_skip(&mut policy_skipped, reason);
            }
        }
    }
    (selected_files, policy_skipped)
}
