use anyhow::{Context, Result, bail};
use maestria_code_intel::{CodeQuery, RepositoryCodeIndex};
use maestria_core::InstanceManifest;
use std::path::{Path, PathBuf};

const PARSER_GENERATION: &str = "cargo-rust-code-v1";
const MAX_QUERY_LIMIT: usize = 1_000;
const INDEX_FILENAME: &str = "repository-code-index.json";

pub(crate) fn run_index(instance_dir: PathBuf, repository: PathBuf) -> Result<()> {
    let layout = super::super::helpers::validated_instance(instance_dir)?;
    let manifest = super::super::helpers::load_manifest(&layout)?;
    let repository = allowed_repository_root(&repository, &manifest)?;
    let index = RepositoryCodeIndex::build_with_exclusions(
        &repository,
        PARSER_GENERATION,
        &manifest.excluded_patterns,
    )
    .map_err(|error| anyhow::anyhow!("build repository code index: {error}"))?;
    let index_path = layout.system_dir.join(INDEX_FILENAME);
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
    let index_path = layout.system_dir.join(INDEX_FILENAME);
    let index = RepositoryCodeIndex::load(&index_path)
        .map_err(|error| anyhow::anyhow!("load repository code index: {error}"))?;
    if index.is_stale_generation(PARSER_GENERATION) {
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
    if index
        .is_stale_repository()
        .map_err(|error| anyhow::anyhow!("check repository code index freshness: {error}"))?
    {
        bail!(
            "repository code index is stale for the current repository worktree; run `maestria index repository` again"
        );
    }
    let result = index.query(query, limit);
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
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
        let source = repository.join(&symbol.provenance.file_path);
        let canonical = source
            .canonicalize()
            .with_context(|| format!("canonicalize indexed source {}", source.display()))?;
        if !canonical.starts_with(&repository) || !source_allowed(&canonical, manifest)? {
            bail!(
                "indexed source {} is outside the instance read scope or excluded by privacy policy",
                source.display()
            );
        }
    }
    Ok(())
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
