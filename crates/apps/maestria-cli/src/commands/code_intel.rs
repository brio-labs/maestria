use anyhow::{Context, Result, bail};
use maestria_code_intel::{
    CodeQuery, CommitSha, ContextDirection, MAX_CONTEXT_DEPTH, REPOSITORY_CODE_CANDIDATES_FILENAME,
    REPOSITORY_CODE_INDEX_FILENAME, REPOSITORY_CODE_PARSER_GENERATION, ReferencesDirection,
    RepositoryCodeIndex, RepositoryContextQuery, RepositoryFreshness, RepositoryIndexBuildMode,
    build_or_update_repository_index, is_plausible_commit_sha,
};
use maestria_core::{InstanceLayout, InstanceManifest};
use maestria_governance::AutonomyProfile;
use std::path::{Path, PathBuf};

use super::code_intel_auth::code_intel_authorization;
use super::code_intel_sources::register_repository_sources;

const MAX_QUERY_LIMIT: usize = 1_000;

pub(crate) async fn run_index(instance_dir: PathBuf, repository: PathBuf) -> Result<()> {
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let repository = allowed_repository_root(&repository, &manifest)?;
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    // The mutation session fails closed on an index whose generation, privacy
    // exclusions, or integrity do not match the current instance (daemon
    // runtime construction). The index is regenerable cache, so a stale or
    // corrupt persisted index is removed up front; `build_or_update_repository_index`
    // then takes its Full fallback, which is the one-time migration path.
    repair_stale_repository_index(&index_path, &manifest)?;
    // Instance state is written under the instance write lock (R28/R32): the
    // mutation session is the one owner of startup, recovery, and shutdown.
    let session =
        maestria_daemon::MutationSession::start(layout.clone(), AutonomyProfile::TrustedWorkspace)
            .await
            .context("start mutation session")?;
    let result = async {
        let candidates_path = layout.system_dir.join(REPOSITORY_CODE_CANDIDATES_FILENAME);
        let mut index = build_or_update_repository_index(
            &index_path,
            &candidates_path,
            &repository,
            REPOSITORY_CODE_PARSER_GENERATION,
            &manifest.excluded_patterns,
        )
        .map_err(|error| anyhow::anyhow!("build repository code index: {error}"))?;
        let mut mode = index.1;
        if !matches!(mode, RepositoryIndexBuildMode::Noop) {
            index
                .0
                .save(&index_path)
                .map_err(|error| anyhow::anyhow!("save repository code index: {error}"))?;
        }
        // Register every indexed source as a canonical artifact through the
        // kernel so code queries can authorize symbols against durable
        // evidence. If a file changed between extraction and registration
        // (content hash mismatch), rebuild once and re-register; the
        // incremental path re-extracts the mismatched files.
        let mismatched = register_repository_sources(&layout, &session, &index.0, &repository)
            .await
            .map_err(|error| anyhow::anyhow!("register repository code sources: {error}"))?;
        if !mismatched.is_empty() {
            let rebuilt = build_or_update_repository_index(
                &index_path,
                &candidates_path,
                &repository,
                REPOSITORY_CODE_PARSER_GENERATION,
                &manifest.excluded_patterns,
            )
            .map_err(|error| anyhow::anyhow!("rebuild repository code index: {error}"))?;
            mode = rebuilt.1;
            if !matches!(mode, RepositoryIndexBuildMode::Noop) {
                rebuilt
                    .0
                    .save(&index_path)
                    .map_err(|error| anyhow::anyhow!("save repository code index: {error}"))?;
            }
            index = rebuilt;
            let remaining = register_repository_sources(&layout, &session, &index.0, &repository)
                .await
                .map_err(|error| anyhow::anyhow!("register repository code sources: {error}"))?;
            if !remaining.is_empty() {
                // The repository changed again mid-command; the next index
                // run reconciles it. The persisted index is still consistent
                // with the worktree at save time.
                eprintln!(
                    "warning: {} repository source(s) changed during indexing; re-run `maestria index repository` to reconcile",
                    remaining.len()
                );
            }
        }
        Ok::<_, anyhow::Error>((index_path, mode, index.0.summary))
    }
    .await;
    let (index_path, mode, summary) = session.finish(result).await?;
    println!("repository_code_index={}", index_path.display());
    println!("mode={}", mode.as_str());
    for warning in &summary.workspace_warnings {
        eprintln!("warning: {warning}");
    }
    println!(
        "changed_files={} changed_symbols={}",
        summary.changed.files.len(),
        summary.changed.symbols.len()
    );
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

/// Remove a persisted repository code index that the daemon runtime would
/// reject (stale parser generation, changed privacy exclusions, or integrity
/// failure). Returns Ok for a missing or healthy index.
fn repair_stale_repository_index(index_path: &Path, manifest: &InstanceManifest) -> Result<()> {
    if !index_path.exists() {
        return Ok(());
    }
    let unhealthy = match RepositoryCodeIndex::load(index_path) {
        Ok(index) => {
            index.is_stale_generation(REPOSITORY_CODE_PARSER_GENERATION)
                || index.summary.excluded_patterns != manifest.excluded_patterns
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

pub(crate) fn run_search(instance_dir: PathBuf, query: CodeQuery, limit: usize) -> Result<()> {
    if !(1..=MAX_QUERY_LIMIT).contains(&limit) {
        bail!("code query limit must be between 1 and {MAX_QUERY_LIMIT}");
    }
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let index = load_verified_code_index(&layout, &manifest)?;
    let authorization = code_intel_authorization(&layout)?;
    let result = index.query(query, limit, |symbol| {
        authorization
            .resolver
            .authorizes(symbol, &authorization.context)
    })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

pub(crate) fn run_changed(
    instance_dir: PathBuf,
    since: Option<String>,
    limit: usize,
) -> Result<()> {
    if !(1..=MAX_QUERY_LIMIT).contains(&limit) {
        bail!("code query limit must be between 1 and {MAX_QUERY_LIMIT}");
    }
    let since = match since {
        Some(reference) => {
            if !is_plausible_commit_sha(&reference) {
                bail!("invalid commit reference for --since: {reference}");
            }
            Some(CommitSha::new(reference))
        }
        None => None,
    };
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let index = load_verified_code_index(&layout, &manifest)?;
    let authorization = code_intel_authorization(&layout)?;
    let result = index.query(CodeQuery::Changed { since }, limit, |symbol| {
        authorization
            .resolver
            .authorizes(symbol, &authorization.context)
    })?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

pub(crate) fn run_references(
    instance_dir: PathBuf,
    pattern: String,
    direction: ReferencesDirection,
    limit: usize,
) -> Result<()> {
    if !(1..=MAX_QUERY_LIMIT).contains(&limit) {
        bail!("code query limit must be between 1 and {MAX_QUERY_LIMIT}");
    }
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let index = load_verified_code_index(&layout, &manifest)?;
    let authorization = code_intel_authorization(&layout)?;
    let result = index.references(
        CodeQuery::References { pattern, direction },
        limit,
        |symbol| {
            authorization
                .resolver
                .authorizes(symbol, &authorization.context)
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

pub(crate) fn run_context(
    instance_dir: PathBuf,
    pattern: String,
    depth: usize,
    nodes: usize,
    direction: String,
) -> Result<()> {
    if !(1..=MAX_QUERY_LIMIT).contains(&nodes) {
        bail!("context node limit must be between 1 and {MAX_QUERY_LIMIT}");
    }
    if !(1..=MAX_CONTEXT_DEPTH).contains(&depth) {
        bail!("context depth must be between 1 and {MAX_CONTEXT_DEPTH}");
    }
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let index = load_verified_code_index(&layout, &manifest)?;
    let direction = parse_context_direction(&direction)?;
    let authorization = code_intel_authorization(&layout)?;
    let result = index.context(
        RepositoryContextQuery {
            query: CodeQuery::Symbol { pattern },
            direction,
            relation_kinds: None,
            max_depth: depth,
            max_nodes: nodes,
        },
        |symbol| {
            authorization
                .resolver
                .authorizes(symbol, &authorization.context)
        },
    )?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

/// Load the repository code index and verify generation, privacy exclusions, scope,
/// freshness, and provenance in one phase so search and context handlers only own
/// authorization and query execution (R17/R20).
fn load_verified_code_index(
    layout: &InstanceLayout,
    manifest: &InstanceManifest,
) -> Result<RepositoryCodeIndex> {
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    let index = RepositoryCodeIndex::load(&index_path)
        .map_err(|error| anyhow::anyhow!("load repository code index: {error}"))?;
    if index.is_stale_generation(REPOSITORY_CODE_PARSER_GENERATION) {
        bail!(
            "repository code index uses a stale parser generation; run `maestria index repository` again"
        );
    }
    if index.summary.excluded_patterns != manifest.excluded_patterns {
        bail!(
            "repository code index uses stale privacy exclusions; run `maestria index repository` again"
        );
    }
    validate_index_scope(&index, manifest)?;
    ensure_fresh(&index)?;
    index
        .validate_provenance()
        .map_err(|error| anyhow::anyhow!("validate repository code index integrity: {error}"))?;
    Ok(index)
}

fn ensure_fresh(index: &RepositoryCodeIndex) -> Result<()> {
    if let RepositoryFreshness::Stale { indexed, current } = index
        .freshness()
        .map_err(|error| anyhow::anyhow!("check repository code index freshness: {error}"))?
    {
        bail!(
            "repository code index is stale (indexed commit {}, current commit {}, indexed worktree {}, current worktree {})",
            indexed.commit_sha,
            current.commit_sha,
            indexed.worktree_identity,
            current.worktree_identity
        );
    }
    Ok(())
}

fn parse_context_direction(direction: &str) -> Result<ContextDirection> {
    match direction {
        "outgoing" => Ok(ContextDirection::Outgoing),
        "incoming" => Ok(ContextDirection::Incoming),
        "both" => Ok(ContextDirection::Both),
        _ => bail!("context direction must be outgoing, incoming, or both"),
    }
}

fn allowed_repository_root(repository: &Path, manifest: &InstanceManifest) -> Result<PathBuf> {
    let repository = repository
        .canonicalize()
        .with_context(|| format!("canonicalize repository {}", repository.display()))?;
    if !repository.is_dir() {
        bail!(
            "repository path is not a directory: {}",
            repository.display()
        );
    }
    if path_excluded(&repository, &manifest.excluded_patterns) {
        bail!(
            "repository {} is outside the instance read scope or excluded by privacy policy",
            repository.display()
        );
    }
    let allowed = manifest
        .read_roots
        .iter()
        .map(|root| root.canonicalize())
        .collect::<Result<Vec<_>, _>>();
    let allowed = allowed.context("canonicalize configured read roots")?;
    if allowed.iter().any(|root| repository.starts_with(root)) {
        Ok(repository)
    } else {
        bail!(
            "repository {} is outside the instance read scope",
            repository.display()
        );
    }
}

fn validate_index_scope(index: &RepositoryCodeIndex, manifest: &InstanceManifest) -> Result<()> {
    let repository = allowed_repository_root(Path::new(&index.summary.repository_root), manifest)?;
    for symbol in &index.symbols {
        let relative_source = Path::new(&symbol.provenance.file_path);
        if !is_safe_relative_path(relative_source) {
            bail!(
                "indexed source {} is outside the instance read scope or excluded by privacy policy",
                relative_source.display()
            );
        }
        let source = repository.join(relative_source);
        let canonical = match source.canonicalize() {
            Ok(canonical) => canonical,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let canonical_parent = canonical_existing_parent(&source)?;
                if !source.starts_with(&repository)
                    || path_excluded(&source, &manifest.excluded_patterns)
                    || !canonical_parent.starts_with(&repository)
                    || !source_allowed(&canonical_parent, manifest)?
                {
                    bail!(
                        "indexed source {} is outside the instance read scope or excluded by privacy policy",
                        source.display()
                    );
                }
                continue;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("canonicalize indexed source {}", source.display()));
            }
        };
        if !canonical.starts_with(&repository) || !source_allowed(&canonical, manifest)? {
            bail!(
                "indexed source {} is outside the instance read scope or excluded by privacy policy",
                source.display()
            );
        }
    }
    Ok(())
}

fn canonical_existing_parent(path: &Path) -> Result<PathBuf> {
    let mut candidate = path;
    loop {
        match candidate.canonicalize() {
            Ok(canonical) => return Ok(canonical),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = candidate.parent() else {
                    bail!("cannot resolve an existing parent for {}", path.display());
                };
                candidate = parent;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("canonicalize indexed source {}", path.display()));
            }
        }
    }
}

fn source_allowed(path: &Path, manifest: &InstanceManifest) -> Result<bool> {
    if path_excluded(path, &manifest.excluded_patterns) {
        return Ok(false);
    }
    let roots = manifest
        .read_roots
        .iter()
        .map(|root| root.canonicalize())
        .collect::<Result<Vec<_>, _>>()
        .context("canonicalize configured read roots")?;
    Ok(roots.iter().any(|root| path.starts_with(root)))
}

fn is_safe_relative_path(path: &Path) -> bool {
    path.is_relative()
        && path
            .components()
            .all(|component| !matches!(component, std::path::Component::ParentDir))
}

fn path_excluded(path: &Path, patterns: &[String]) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        name == ".git"
            || name == ".ssh"
            || name == ".gnupg"
            || name == "secrets"
            || name == "node_modules"
            || name == "target"
            || name == "dist"
            || name == "build"
            || patterns.iter().any(|pattern| {
                pattern.as_str() == name
                    || (pattern == ".env.*" && name.starts_with(".env."))
                    || (pattern == "*.pem" && name.ends_with(".pem"))
                    || (pattern == "*.key" && name.ends_with(".key"))
            })
    })
}
