//! Repository code index command: selection resolution (candidate-tree
//! approval or explicit includes), the selection-scoped build, source
//! registration, and the candidate-tree listing.

use anyhow::{Context, Result, bail};
use clap::Args;
use maestria_code_intel::{
    REPOSITORY_CODE_CANDIDATES_FILENAME, REPOSITORY_CODE_INDEX_FILENAME,
    REPOSITORY_CODE_PARSER_GENERATION, RepositoryCodeIndex, RepositoryIndexBuildMode,
    RepositorySelection, build_or_update_repository_index,
};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_governance::AutonomyProfile;
use maestria_index_selection::{
    CandidateDir, Class, IndexPolicy, IndexSelectionProfile, bound_candidate_tree, save_profile,
    scan_repository_candidates,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::code_intel::allowed_repository_root;

/// Selection flags for `index repository`: which directories to index and
/// under which per-directory policies. Flattened into the `Repository`
/// clap variant so the command module owns the selection vocabulary.
#[derive(Debug, Args)]
pub struct RepositoryIndexArgs {
    /// Print the bounded classified candidate tree as JSON and exit (no
    /// instance needed).
    #[arg(long)]
    pub list: bool,
    /// Scripted approval: include every Recommended/Maybe directory,
    /// exclude Noise (ignored when --include is given).
    #[arg(long)]
    pub yes: bool,
    /// Repository-relative directory to index; repeatable. The root itself
    /// selects the whole repository.
    #[arg(long = "include")]
    pub include: Vec<PathBuf>,
    /// Write the selection to system/repository-index-selection.json.
    #[arg(long)]
    pub save_selection: bool,
    /// Skip files larger than N bytes; 0 disables.
    #[arg(long)]
    pub max_file_bytes: Option<u64>,
    /// Skip minified single-line bundles.
    #[arg(long)]
    pub skip_minified: bool,
    /// Index the whole repository (the default indexes the classifier's
    /// Recommended directories only).
    #[arg(long)]
    pub all: bool,
}

pub(crate) async fn run_index(
    instance_dir: PathBuf,
    repository: PathBuf,
    args: RepositoryIndexArgs,
) -> Result<()> {
    // --list: print the bounded classified candidate tree and exit. No
    // instance, no session, no selection state is touched.
    if args.list {
        print_candidates(&repository)?;
        return Ok(());
    }
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let repository = allowed_repository_root(&repository, &manifest)?;
    let (selection, policies) = resolve_repository_selection(
        &repository,
        &args.include,
        args.all,
        args.yes,
        args.max_file_bytes,
        args.skip_minified,
    )?;
    if args.save_selection {
        save_repository_selection(&layout, &repository, &selection, &policies)?;
    }
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    // The mutation session fails closed on an index whose generation, privacy
    // exclusions, selection, or integrity do not match the current instance
    // (daemon runtime construction). The index is regenerable cache, so a
    // stale or corrupt persisted index is removed up front;
    // `build_or_update_repository_index` then takes its Full fallback, which
    // is the one-time migration path.
    repair_stale_repository_index(&index_path, &manifest, &selection, &policies)?;
    // Instance state is written under the instance write lock (R28/R32): the
    // mutation session is the one owner of startup, recovery, and shutdown.
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await
            .context("start mutation session")?;
    let result = async {
        let candidates_path = layout.system_dir.join(REPOSITORY_CODE_CANDIDATES_FILENAME);
        let build = RepositoryBuild {
            excluded_patterns: &manifest.excluded_patterns,
            selection: &selection,
            policies: &policies,
            index_path: &index_path,
            candidates_path: &candidates_path,
        };
        let (mode, summary) =
            build_and_register_sources(&layout, &session, &repository, &build).await?;
        Ok::<_, anyhow::Error>((index_path, mode, summary))
    }
    .await;
    let (index_path, mode, summary) = session.finish(result).await?;
    print_index_result(&index_path, mode, &summary)?;
    Ok(())
}

/// The build inputs shared by the initial build and the rebuild-on-
/// registration-mismatch pass.
struct RepositoryBuild<'a> {
    excluded_patterns: &'a [String],
    selection: &'a RepositorySelection,
    policies: &'a BTreeMap<String, IndexPolicy>,
    index_path: &'a Path,
    candidates_path: &'a Path,
}

/// Build the index over the selection, register every indexed source as a
/// canonical artifact through the kernel, and rebuild once and re-register
/// when a file changed between extraction and registration (the
/// incremental path re-extracts the mismatched files). Returns the final
/// mode and summary.
async fn build_and_register_sources(
    layout: &InstanceLayout,
    session: &maestria_daemon::MutationSession,
    repository: &Path,
    build: &RepositoryBuild<'_>,
) -> Result<(
    RepositoryIndexBuildMode,
    maestria_code_intel::CodeIndexSummary,
)> {
    let build_index = || {
        build_or_update_repository_index(
            build.index_path,
            build.candidates_path,
            repository,
            REPOSITORY_CODE_PARSER_GENERATION,
            build.excluded_patterns,
            build.selection,
            build.policies,
        )
        .map_err(|error| anyhow::anyhow!("build repository code index: {error}"))
    };
    let build_started = maestria_retrieval::MonotonicInstant::now();
    let mut index = build_index()?;
    let mut mode = index.1;
    if !matches!(mode, RepositoryIndexBuildMode::Noop) {
        println!(
            "built repository code index in {} (mode={mode:?})",
            super::index_metrics::format_duration(build_started.elapsed())
        );
        index
            .0
            .save(build.index_path)
            .map_err(|error| anyhow::anyhow!("save repository code index: {error}"))?;
    }
    let (mismatched, _) = maestria_daemon::register_repository_sources_with_session(
        layout, session, &index.0, repository,
    )
    .await
    .map_err(|error| anyhow::anyhow!("register repository code sources: {error}"))?;
    if !mismatched.is_empty() {
        // The repository changed between extraction and registration:
        // rebuild once and re-register.
        let rebuilt = build_index()?;
        mode = rebuilt.1;
        if !matches!(mode, RepositoryIndexBuildMode::Noop) {
            rebuilt
                .0
                .save(build.index_path)
                .map_err(|error| anyhow::anyhow!("save repository code index: {error}"))?;
        }
        index = rebuilt;
        let (remaining, _) = maestria_daemon::register_repository_sources_with_session(
            layout, session, &index.0, repository,
        )
        .await
        .map_err(|error| anyhow::anyhow!("register repository code sources: {error}"))?;
        if !remaining.is_empty() {
            // The repository changed again mid-command; the next index run
            // reconciles it. The persisted index is still consistent with
            // the worktree at save time.
            eprintln!(
                "warning: {} repository source(s) changed during indexing; re-run \
                 `maestria index repository` to reconcile",
                remaining.len()
            );
        }
    }
    Ok((mode, index.0.summary))
}

/// Resolve the repository selection and per-directory policies from the CLI
/// flags. Precedence: `--include` > `--all` > `--yes` > the default.
///
/// - `--include` entries: absolute-under-root paths are validated (rejected
///   outside the root) and stripped to repository relative; the root itself
///   selects the whole repository. When any policy flag is given, every
///   included directory runs under the batch policy; otherwise no policy
///   overrides apply.
/// - `--all`: the whole repository, no policies.
/// - `--yes`: scripted approval — every `Recommended`/`Maybe` directory is
///   included with its classification default policy, `Noise` subtrees
///   excluded.
/// - Default: the classifier's recommendation — `Recommended` directories
///   only (the same pre-checked set the Studio shows), so a plain run skips
///   generated dumps and uncertain directories without asking.
fn resolve_repository_selection(
    repository: &Path,
    includes: &[PathBuf],
    all: bool,
    yes: bool,
    max_file_bytes: Option<u64>,
    skip_minified: bool,
) -> Result<(RepositorySelection, BTreeMap<String, IndexPolicy>)> {
    if !includes.is_empty() {
        let mut relative_paths = Vec::new();
        for include in includes {
            // Repository-relative includes resolve against the repository
            // root; absolute ones are used as-is.
            let include = if include.is_absolute() {
                include.clone()
            } else {
                repository.join(include)
            };
            let include = include
                .canonicalize()
                .with_context(|| format!("canonicalize --include path {}", include.display()))?;
            if !include.starts_with(repository) {
                bail!(
                    "--include path {} is outside the repository {}",
                    include.display(),
                    repository.display()
                );
            }
            if include == repository {
                return Ok((RepositorySelection::everything(), BTreeMap::new()));
            }
            relative_paths.push(
                include
                    .strip_prefix(repository)
                    .map_err(|_| anyhow::anyhow!("validated include escapes its root"))?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let selection = RepositorySelection::try_new(relative_paths)?;
        let policies = if max_file_bytes.is_some() || skip_minified {
            let mut batch = IndexPolicy::everything();
            batch.skip_minified = skip_minified;
            if let Some(bytes) = max_file_bytes {
                batch.max_file_bytes = bytes;
            }
            selection
                .as_paths()
                .map(|path| (path.to_string(), batch))
                .collect()
        } else {
            BTreeMap::new()
        };
        return Ok((selection, policies));
    }
    if all {
        return Ok((RepositorySelection::everything(), BTreeMap::new()));
    }
    let tree = scan_repository_candidates(repository)?;
    let mut approved = Vec::new();
    for child in &tree.children {
        if yes {
            collect_scripted_approval(child, &mut approved);
        } else {
            collect_recommended_only(child, &mut approved);
        }
    }
    let mut relative_paths = Vec::new();
    let mut policies = BTreeMap::new();
    for (path, policy) in approved {
        let relative = path
            .strip_prefix(repository)
            .map_err(|_| anyhow::anyhow!("candidate tree is rooted at the repository"))?
            .to_string_lossy()
            .into_owned();
        relative_paths.push(relative.clone());
        if yes {
            policies.insert(relative, policy);
        }
    }
    Ok((RepositorySelection::try_new(relative_paths)?, policies))
}

/// Scripted repository approval: include every `Recommended`/`Maybe`
/// directory under `node` (mirroring `approve_scripted` semantics: `Noise`
/// subtrees excluded, recursion stops at `Recommended`), recording each
/// approved path with its classification default policy.
fn collect_scripted_approval(node: &CandidateDir, approved: &mut Vec<(PathBuf, IndexPolicy)>) {
    if node.class == Class::Noise {
        return;
    }
    approved.push((node.path.clone(), node.policy));
    if node.class == Class::Recommended {
        return;
    }
    for child in &node.children {
        collect_scripted_approval(child, approved);
    }
}

/// The default repository approval: `Recommended` directories only,
/// recursing through every other class to find them (mirroring the Studio
/// pre-check semantics). `Maybe`/`Noise` directories are left out unless
/// explicitly approved.
fn collect_recommended_only(node: &CandidateDir, approved: &mut Vec<(PathBuf, IndexPolicy)>) {
    if node.class == Class::Recommended {
        approved.push((node.path.clone(), node.policy));
        return;
    }
    for child in &node.children {
        collect_recommended_only(child, approved);
    }
}

/// Print the bounded classified candidate tree for `repository`.
fn print_candidates(repository: &Path) -> Result<()> {
    let root = repository
        .canonicalize()
        .context("canonicalize repository for candidate listing")?;
    if !root.is_dir() {
        bail!("repository path is not a directory: {}", root.display());
    }
    let mut tree = scan_repository_candidates(&root)?;
    bound_candidate_tree(&mut tree);
    println!("{}", serde_json::to_string_pretty(&tree)?);
    Ok(())
}

/// Persist the resolved selection as the instance's repository selection
/// profile (`system/repository-index-selection.json`).
fn save_repository_selection(
    layout: &InstanceLayout,
    repository: &Path,
    selection: &RepositorySelection,
    policies: &BTreeMap<String, IndexPolicy>,
) -> Result<()> {
    let profile = IndexSelectionProfile {
        root: repository.to_path_buf(),
        includes: selection.as_paths().map(PathBuf::from).collect::<Vec<_>>(),
        policies: policies
            .iter()
            .map(|(path, policy)| (PathBuf::from(path), *policy))
            .collect(),
    };
    save_profile(
        &layout.system_dir.join("repository-index-selection.json"),
        &profile,
    )
}

/// Print the index result lines, including the recorded selection.
fn print_index_result(
    index_path: &Path,
    mode: RepositoryIndexBuildMode,
    summary: &maestria_code_intel::CodeIndexSummary,
) -> Result<()> {
    println!("repository_code_index={}", index_path.display());
    println!("mode={}", mode.as_str());
    println!(
        "selected_paths={}",
        if summary.selected_paths.is_empty() {
            "whole-repo".to_string()
        } else {
            summary.selected_paths.join(",")
        }
    );
    for warning in &summary.workspace_warnings {
        eprintln!("warning: {warning}");
    }
    println!(
        "changed_files={} changed_symbols={}",
        summary.changed.files().len(),
        summary.changed.symbols().len()
    );
    println!("{}", serde_json::to_string_pretty(summary)?);
    Ok(())
}

/// Remove a persisted repository code index that the daemon runtime would
/// reject (stale parser generation, changed privacy exclusions, a
/// selection/policy mismatch, or integrity failure). Returns Ok for a
/// missing or healthy index.
fn repair_stale_repository_index(
    index_path: &Path,
    manifest: &InstanceManifest,
    selection: &RepositorySelection,
    policies: &BTreeMap<String, IndexPolicy>,
) -> Result<()> {
    if !index_path.exists() {
        return Ok(());
    }
    let unhealthy = match RepositoryCodeIndex::load(index_path) {
        Ok(index) => {
            index.is_stale_generation(REPOSITORY_CODE_PARSER_GENERATION)
                || index.summary.excluded_patterns != manifest.excluded_patterns
                || index.summary.selected_paths != selection.as_paths().collect::<Vec<_>>()
                || index.summary.selection_policies != *policies
        }
        Err(_) => true,
    };
    if unhealthy {
        std::fs::remove_file(index_path).with_context(|| {
            format!(
                "remove stale repository code index {}",
                index_path.display()
            )
        })?;
    }
    Ok(())
}
