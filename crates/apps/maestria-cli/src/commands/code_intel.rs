use anyhow::{Context, Result, bail};
use maestria_code_intel::{
    CodeQuery, ContextDirection, REPOSITORY_CODE_INDEX_FILENAME, REPOSITORY_CODE_PARSER_GENERATION,
    RepositoryCodeIndex, RepositoryContextQuery, RepositoryFreshness,
};
use maestria_core::InstanceManifest;
use std::path::{Path, PathBuf};

const MAX_QUERY_LIMIT: usize = 1_000;

pub(crate) fn run_index(instance_dir: PathBuf, repository: PathBuf) -> Result<()> {
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let repository = allowed_repository_root(&repository, &manifest)?;
    let index = RepositoryCodeIndex::build_with_exclusions(
        &repository,
        REPOSITORY_CODE_PARSER_GENERATION,
        &manifest.excluded_patterns,
    )
    .map_err(|error| anyhow::anyhow!("build repository code index: {error}"))?;
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    index
        .save(&index_path)
        .map_err(|error| anyhow::anyhow!("save repository code index: {error}"))?;
    println!("repository_code_index={}", index_path.display());
    println!("{}", serde_json::to_string_pretty(&index.summary)?);
    Ok(())
}

pub(crate) fn run_search(instance_dir: PathBuf, query: CodeQuery, limit: usize) -> Result<()> {
    if !(1..=MAX_QUERY_LIMIT).contains(&limit) {
        bail!("code query limit must be between 1 and {MAX_QUERY_LIMIT}");
    }
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
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
    validate_index_scope(&index, &manifest)?;
    ensure_fresh(&index)?;
    index
        .validate_provenance()
        .map_err(|error| anyhow::anyhow!("validate repository code index integrity: {error}"))?;
    let result = index.query(query, limit);
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
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let index_path = layout.system_dir.join(REPOSITORY_CODE_INDEX_FILENAME);
    let index = RepositoryCodeIndex::load(&index_path)
        .map_err(|error| anyhow::anyhow!("load repository code index: {error}"))?;
    if index.is_stale_generation(REPOSITORY_CODE_PARSER_GENERATION) {
        bail!("repository code index uses a stale parser generation");
    }
    if index.summary.excluded_patterns != manifest.excluded_patterns {
        bail!("repository code index uses stale privacy exclusions");
    }
    validate_index_scope(&index, &manifest)?;
    ensure_fresh(&index)?;
    index
        .validate_provenance()
        .map_err(|error| anyhow::anyhow!("validate repository code index integrity: {error}"))?;
    let direction = parse_context_direction(&direction)?;
    let result = index.context(RepositoryContextQuery {
        query: CodeQuery::Symbol { pattern },
        direction,
        relation_kinds: None,
        max_depth: depth,
        max_nodes: nodes,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
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
