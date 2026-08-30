use anyhow::{Result, anyhow};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_governance::{PrivacyExclusions, Scope};
use maestria_index_selection::{IndexPolicy, Selection};
use std::{
    fs,
    io::IsTerminal,
    path::{Path, PathBuf},
    time::Duration,
};

use super::index_batch::{
    BatchTallies, ProcessContext, SUBMIT_WINDOW, SubmitOutcome, await_file_terminal,
    drain_recovery, drain_validation_recovery, submit_file,
};
use super::index_metrics::IndexMetrics;
use super::index_selection::{SelectionPlan, approve_interactively, approve_scripted, record_skip};
use crate::helpers;

/// Index every selected file under per-file supervision, wait for recovery
/// work, and print the run summary with live metrics. Returns `Ok(())` when
/// no selected file failed.
async fn run_selected_batch(
    selected_files: &[PathBuf],
    ctx: &ProcessContext<'_>,
    recovery: &maestria_daemon::RecoveryQueue,
    policy_skipped_total: usize,
) -> Result<()> {
    let mut metrics = IndexMetrics::new(selected_files.len(), ctx.layout)?;
    let mut tallies = BatchTallies::new();
    for window in selected_files.chunks(SUBMIT_WINDOW) {
        let mut in_flight = Vec::with_capacity(window.len());
        for (offset, file) in window.iter().enumerate() {
            let prefix = format!(
                "[{}/{}]",
                tallies.completed + offset + 1,
                selected_files.len()
            );
            match submit_file(file, prefix, ctx).await {
                Ok(SubmitOutcome::Terminal(outcome, bytes)) => {
                    tallies.record(outcome, bytes, &mut metrics);
                }
                Ok(SubmitOutcome::InFlight(pending)) => in_flight.push(pending),
                Err(error) => {
                    eprintln!("failed artifact path={} error={error}", file.display());
                    tallies.failed += 1;
                    tallies.completed += 1;
                }
            }
        }
        for pending in in_flight {
            let bytes = pending.bytes;
            let outcome = await_file_terminal(pending, ctx).await;
            tallies.record(outcome, bytes, &mut metrics);
        }
        if let Some(line) = metrics.status_line(tallies.completed) {
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
        metrics.summary(
            tallies.indexed,
            tallies.unchanged,
            tallies.skipped + policy_skipped_total,
            tallies.failed,
        )
    );
    if tallies.failed > 0 {
        Err(anyhow!(
            "{} of {} selected files failed to index",
            tallies.failed,
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
