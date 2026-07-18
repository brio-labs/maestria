use super::{CodeIntelError, RepositoryCodeIndex};
use crate::identity::discover_repository_identity;
use crate::metadata::extract_workspace_packages;
use crate::symbols::extract_symbols;
use std::collections::BTreeSet;
use std::path::Path;

impl RepositoryCodeIndex {
    /// Build a fresh index for `root` using `parser_generation`.
    pub fn build(
        root: impl AsRef<Path>,
        parser_generation: impl Into<String>,
    ) -> Result<Self, CodeIntelError> {
        Self::build_with_exclusions(root, parser_generation, &[])
    }

    /// Build an index while applying manifest-compatible path exclusions.
    pub fn build_with_exclusions(
        root: impl AsRef<Path>,
        parser_generation: impl Into<String>,
        excluded_patterns: &[String],
    ) -> Result<Self, CodeIntelError> {
        let root = root.as_ref();
        let parser_generation = parser_generation.into();
        let initial_identity = discover_repository_identity(root, excluded_patterns)?;
        let mut packages = extract_workspace_packages(
            Path::new(&initial_identity.root),
            &initial_identity,
            &parser_generation,
            excluded_patterns,
        )?;
        let identity = discover_repository_identity(root, excluded_patterns)?;
        if identity.commit != initial_identity.commit
            || identity.worktree_identity != initial_identity.worktree_identity
        {
            packages = extract_workspace_packages(
                Path::new(&identity.root),
                &identity,
                &parser_generation,
                excluded_patterns,
            )?;
        }
        let (symbols, relations, relation_summary) = extract_symbols(
            &packages,
            Path::new(&identity.root),
            &identity,
            &parser_generation,
            excluded_patterns,
        )?;

        let files = symbols
            .iter()
            .map(|symbol| symbol.provenance.file_path.clone())
            .collect::<BTreeSet<_>>();
        Ok(Self {
            summary: super::types::CodeIndexSummary {
                repository_root: identity.root,
                commit_sha: identity.commit,
                worktree_identity: identity.worktree_identity,
                parser_generation,
                package_count: packages.len(),
                target_count: packages.iter().map(|package| package.targets.len()).sum(),
                symbol_count: symbols.len(),
                file_count: files.len(),
                packages: packages
                    .iter()
                    .map(|package| package.name.clone())
                    .collect(),
                excluded_patterns: excluded_patterns.to_vec(),
                relation_summary,
            },
            packages,
            symbols,
            relations,
        })
    }

    /// Whether stored parser generation or source identity is stale.
    pub fn is_stale_repository(&self) -> Result<bool, CodeIntelError> {
        let identity = discover_repository_identity(
            Path::new(&self.summary.repository_root),
            &self.summary.excluded_patterns,
        )?;
        Ok(identity.commit != self.summary.commit_sha
            || identity.worktree_identity != self.summary.worktree_identity)
    }
}
