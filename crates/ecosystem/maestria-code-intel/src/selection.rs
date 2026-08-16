//! Repository selection domain: which repository-relative directories
//! are indexed, and the per-directory policy gate applied to files.

use maestria_index_selection::{IndexPolicy, Selection, select_source};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

/// A set of repository-relative directory paths to index.
///
/// Invariant: an empty set means the whole repository; otherwise a
/// sorted, deduped set of repository-relative directory paths with no
/// `..`, no absolute/root/prefix components, no empty components, and no
/// trailing slashes (R56). Selection is build configuration: identity,
/// delta, records, and freshness are all scoped to it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositorySelection {
    paths: BTreeSet<String>,
}

impl RepositorySelection {
    /// The whole-repository selection: no paths restricted.
    pub fn everything() -> Self {
        Self {
            paths: BTreeSet::new(),
        }
    }

    /// Builds a selection from repository-relative directory paths.
    ///
    /// Absolute paths, `..`, `/`-rooted, drive-prefixed, and `.`
    /// components are rejected; trailing slashes are trimmed; empty
    /// strings are dropped; duplicates collapse (sorted set semantics).
    pub fn try_new(includes: Vec<String>) -> Result<Self, RepositorySelectionError> {
        let mut paths = BTreeSet::new();
        for include in includes {
            let trimmed = include.trim_end_matches('/');
            if trimmed.is_empty() {
                // Empty after normalization: drop (whole-repo semantics
                // keep `[]`).
                continue;
            }
            if Path::new(trimmed).is_absolute() {
                return Err(RepositorySelectionError::AbsolutePath(include));
            }
            for component in Path::new(trimmed).components() {
                match component {
                    Component::ParentDir => {
                        return Err(RepositorySelectionError::ParentComponent(include));
                    }
                    Component::RootDir => {
                        return Err(RepositorySelectionError::RootComponent(include));
                    }
                    Component::Prefix(_) => {
                        return Err(RepositorySelectionError::AbsolutePath(include));
                    }
                    Component::CurDir => return Err(RepositorySelectionError::EmptyPath),
                    Component::Normal(_) => {}
                }
            }
            paths.insert(trimmed.to_string());
        }
        Ok(Self { paths })
    }

    /// Whether `relative` (repository-relative path) is covered: the
    /// whole repository, or a directory equal to or an ancestor of it.
    pub fn contains(&self, relative: &str) -> bool {
        if self.is_whole() {
            return true;
        }
        self.paths
            .iter()
            .any(|p| relative == p || relative.starts_with(&format!("{p}/")))
    }

    /// Whether no directory restriction applies (whole repository).
    pub fn is_whole(&self) -> bool {
        self.paths.is_empty()
    }

    /// Sorted selected directory paths (empty for the whole repository).
    pub fn as_paths(&self) -> impl Iterator<Item = &str> {
        self.paths.iter().map(String::as_str)
    }
}

impl TryFrom<Vec<String>> for RepositorySelection {
    type Error = RepositorySelectionError;

    fn try_from(paths: Vec<String>) -> Result<Self, Self::Error> {
        Self::try_new(paths)
    }
}

/// Failure while building a [`RepositorySelection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositorySelectionError {
    /// The path is absolute; only repository-relative paths are allowed.
    AbsolutePath(String),
    /// The path contains a `..` component.
    ParentComponent(String),
    /// The path contains a root (`/`) component.
    RootComponent(String),
    /// The path is empty or contains an empty (`.`) component.
    EmptyPath,
}

impl std::fmt::Display for RepositorySelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AbsolutePath(path) => {
                write!(
                    f,
                    "selection path {path:?} is absolute; repository-relative paths only"
                )
            }
            Self::ParentComponent(path) => {
                write!(f, "selection path {path:?} contains a `..` component")
            }
            Self::RootComponent(path) => {
                write!(f, "selection path {path:?} contains a root (`/`) component")
            }
            Self::EmptyPath => write!(f, "selection path is empty or contains a `.` component"),
        }
    }
}

impl std::error::Error for RepositorySelectionError {}

/// Per-file gate combining the selection with per-directory policies.
///
/// A file is indexed only when it is inside the selection and passes the
/// longest-prefix policy (default [`IndexPolicy::everything`]).
pub(crate) struct FileGate {
    selection: RepositorySelection,
    policies: BTreeMap<String, IndexPolicy>,
}

impl FileGate {
    pub(crate) fn new(
        selection: RepositorySelection,
        policies: BTreeMap<String, IndexPolicy>,
    ) -> Self {
        Self {
            selection,
            policies,
        }
    }

    /// Whether `relative` (repository-relative) is indexed under `root`.
    ///
    /// `skip_generated` has no per-file effect (it is whitelist-level,
    /// same as the file-ingestion path). An unreadable file is skipped.
    pub(crate) fn allows(&self, root: &Path, relative: &str) -> bool {
        if !self.selection.contains(relative) {
            return false;
        }
        let path = root.join(relative);
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        let policy = match self
            .policies
            .iter()
            .filter(|(dir, _)| relative == dir.as_str() || relative.starts_with(&format!("{dir}/")))
            .max_by_key(|(dir, _)| dir.matches('/').count())
            .map(|(_, policy)| *policy)
        {
            Some(policy) => policy,
            None => IndexPolicy::everything(),
        };
        matches!(select_source(&path, meta.len(), policy), Selection::Index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_path_rejected() {
        assert_eq!(
            RepositorySelection::try_new(vec!["/abs/path".to_string()]),
            Err(RepositorySelectionError::AbsolutePath(
                "/abs/path".to_string()
            ))
        );
    }

    #[test]
    fn parent_component_rejected() {
        assert_eq!(
            RepositorySelection::try_new(vec!["crates/../lib".to_string()]),
            Err(RepositorySelectionError::ParentComponent(
                "crates/../lib".to_string()
            ))
        );
        assert_eq!(
            RepositorySelection::try_new(vec!["..".to_string()]),
            Err(RepositorySelectionError::ParentComponent("..".to_string()))
        );
    }

    #[test]
    fn cur_component_rejected() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            RepositorySelection::try_new(vec!["./a".to_string()]),
            Err(RepositorySelectionError::EmptyPath)
        );
        // Mid-path dots pass the component check (components() folds them);
        // only a leading `./` still surfaces as CurDir. A bare root
        // normalizes to empty and is dropped.
        let selection = RepositorySelection::try_new(vec!["a/./b".to_string(), "/".to_string()])?;
        assert_eq!(selection.as_paths().collect::<Vec<_>>(), vec!["a/./b"]);
        assert!(selection.contains("a/./b/x.rs"));
        Ok(())
    }

    #[test]
    fn duplicates_collapse_and_trailing_slashes_trim() -> Result<(), Box<dyn std::error::Error>> {
        let selection = RepositorySelection::try_new(vec![
            "crates/one".to_string(),
            "crates/one/".to_string(),
        ])?;
        assert_eq!(selection.as_paths().collect::<Vec<_>>(), vec!["crates/one"]);
        Ok(())
    }

    #[test]
    fn empty_strings_are_dropped() -> Result<(), Box<dyn std::error::Error>> {
        let selection = RepositorySelection::try_new(vec![
            "".to_string(),
            "crates/one".to_string(),
            "/".to_string(),
        ])?;
        assert_eq!(selection.as_paths().collect::<Vec<_>>(), vec!["crates/one"]);
        Ok(())
    }

    #[test]
    fn contains_prefix_semantics() -> Result<(), Box<dyn std::error::Error>> {
        let selection = RepositorySelection::try_new(vec!["crates/foo".to_string()])?;
        assert!(selection.contains("crates/foo"));
        assert!(selection.contains("crates/foo/src"));
        assert!(selection.contains("crates/foo/src/lib.rs"));
        assert!(!selection.contains("crates/foobar"));
        assert!(!selection.contains("crates"));
        assert!(!selection.contains("lib"));
        Ok(())
    }

    #[test]
    fn everything_contains_all() {
        let selection = RepositorySelection::everything();
        assert!(selection.is_whole());
        assert!(selection.contains(""));
        assert!(selection.contains("any/deep/path.rs"));
        assert_eq!(selection.as_paths().count(), 0);
    }

    #[test]
    fn file_gate_applies_selection_and_longest_prefix_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let tmp = tempfile::tempdir()?;
        let root = tmp.path();
        std::fs::create_dir_all(root.join("crates/one"))?;
        std::fs::create_dir_all(root.join("crates/two"))?;
        std::fs::write(root.join("crates/one/lib.rs"), "pub fn one() {}\n")?;
        std::fs::write(root.join("crates/two/lib.rs"), "pub fn two() {}\n")?;

        let selection = RepositorySelection::try_new(vec!["crates/one".to_string()])?;
        let policies = BTreeMap::from([(
            "crates/one".to_string(),
            IndexPolicy {
                max_file_bytes: 10,
                ..IndexPolicy::everything()
            },
        )]);
        let gate = FileGate::new(selection, policies);

        // Outside the selection.
        assert!(!gate.allows(root, "crates/two/lib.rs"));
        // Inside the selection, under the size policy (file is 15 bytes).
        assert!(!gate.allows(root, "crates/one/lib.rs"));
        // Unreadable file is skipped.
        assert!(!gate.allows(root, "crates/one/missing.rs"));

        let gate = FileGate::new(
            RepositorySelection::try_new(vec!["crates".to_string()])?,
            BTreeMap::new(),
        );
        // Whole-prefix directory with default policy.
        assert!(gate.allows(root, "crates/one/lib.rs"));
        assert!(gate.allows(root, "crates/two/lib.rs"));
        Ok(())
    }
}
