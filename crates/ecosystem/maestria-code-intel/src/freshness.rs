use crate::{
    CodeIntelError, RepositoryCodeIndex, RepositorySelection,
    identity::discover_repository_identity, language::active_backends,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Repository identity evidence for an indexed or currently discovered repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryIdentitySnapshot {
    /// Commit identifier produced by `git rev-parse HEAD`.
    pub commit_sha: crate::types::CommitSha,
    /// Deterministic hash of identity-relevant tracked source and manifest content.
    pub worktree_identity: crate::types::WorktreeIdentity,
}

/// Result of comparing persisted index identity with the current repository identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum RepositoryFreshness {
    /// Indexed identity exactly matches current repository identity.
    Current {
        indexed: RepositoryIdentitySnapshot,
        current: RepositoryIdentitySnapshot,
    },
    /// Repository changed since indexing (index and current identities differ).
    Stale {
        indexed: RepositoryIdentitySnapshot,
        current: RepositoryIdentitySnapshot,
    },
}

impl RepositoryCodeIndex {
    /// Compare indexed identity information with the current repository state.
    pub fn freshness(&self) -> Result<RepositoryFreshness, CodeIntelError> {
        let indexed = RepositoryIdentitySnapshot {
            commit_sha: self.summary.commit_sha.clone(),
            worktree_identity: self.summary.worktree_identity.clone(),
        };
        let repository_root = Path::new(&self.summary.repository_root);
        let backends = active_backends(repository_root, &self.summary.excluded_patterns)?;
        // Freshness is scoped to the indexed selection; a malformed
        // persisted selection falls back to whole-repo, which is
        // conservative (the digest then covers more than the index does).
        let selection = match RepositorySelection::try_from(self.summary.selected_paths.clone()) {
            Ok(selection) => selection,
            Err(_) => RepositorySelection::everything(),
        };
        let current = discover_repository_identity(
            repository_root,
            &self.summary.excluded_patterns,
            &backends,
            &selection,
        )?;
        let current = RepositoryIdentitySnapshot {
            commit_sha: current.commit,
            worktree_identity: current.worktree_identity,
        };

        if indexed == current {
            Ok(RepositoryFreshness::Current { indexed, current })
        } else {
            Ok(RepositoryFreshness::Stale { indexed, current })
        }
    }
}
