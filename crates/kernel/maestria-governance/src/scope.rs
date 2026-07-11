use std::path::{Component, Path, PathBuf};

/// Error returned when a path fails lexical containment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainmentError {
    /// The candidate path is empty.
    EmptyPath,
    /// Lexical normalization detected a `..` component that escapes the
    /// filesystem root (or the path starts with `..` and cannot be resolved).
    PathEscapesRoot { path: PathBuf },
    /// The normalized path is not under any configured root.
    PathNotUnderAnyRoot { path: PathBuf },
}

/// Lexically normalise a path without touching the filesystem.
///
/// Resolves `.` and `..` components, returning `None` when a `..` component
/// would escape the filesystem root or the path is empty.
fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut components: Vec<Component<'_>> = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                components.clear();
                components.push(component);
            }
            Component::CurDir => {
                // skip — no effect on the normalized path
            }
            Component::ParentDir => {
                match components.last() {
                    Some(Component::Normal(_)) => {
                        components.pop();
                    }
                    Some(Component::RootDir) | Some(Component::Prefix(_)) => {
                        // `..` at the root — would escape, reject
                        return None;
                    }
                    _ => {
                        // relative path starting with `..` — cannot normalise
                        return None;
                    }
                }
            }
            Component::Normal(_) => {
                components.push(component);
            }
        }
    }

    if components.is_empty() {
        // The path was empty or resolved to nothing (e.g. just `.` and `..`
        // components that cancelled each other on a relative path).
        return None;
    }

    Some(components.iter().collect())
}

/// Check whether `candidate` is lexically contained within at least one
/// of the provided `roots`.
///
/// Both the candidate and every root are normalised lexically (no I/O).
/// An empty candidate or a `..` that escapes the filesystem root produces
/// a `ContainmentError`.
pub fn check_containment(roots: &[PathBuf], candidate: &Path) -> Result<(), ContainmentError> {
    if candidate.as_os_str().is_empty() {
        return Err(ContainmentError::EmptyPath);
    }

    let normalized =
        lexical_normalize(candidate).ok_or_else(|| ContainmentError::PathEscapesRoot {
            path: candidate.to_path_buf(),
        })?;

    for root in roots {
        let normalized_root =
            lexical_normalize(root).ok_or_else(|| ContainmentError::PathEscapesRoot {
                path: root.to_path_buf(),
            })?;

        if normalized.starts_with(&normalized_root) {
            return Ok(());
        }
    }

    Err(ContainmentError::PathNotUnderAnyRoot { path: normalized })
}

// ── Scope ────────────────────────────────────────────────────────────

/// Read/write root configuration for a governed workspace.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scope {
    read_roots: Vec<PathBuf>,
    write_roots: Vec<PathBuf>,
    allowed_harnesses: Vec<String>,
    blocked_commands: Vec<String>,
    web_allowed: bool,
}

impl Scope {
    pub fn new(
        read_roots: Vec<PathBuf>,
        write_roots: Vec<PathBuf>,
        allowed_harnesses: Vec<String>,
        blocked_commands: Vec<String>,
        web_allowed: bool,
    ) -> Self {
        Self {
            read_roots,
            write_roots,
            allowed_harnesses,
            blocked_commands,
            web_allowed,
        }
    }

    // ── existing public surface ──────────────────────────────────

    /// Returns `true` when `path` starts with any read or write root.
    ///
    /// Note: this is a prefix check — it does **not** normalise `..`
    /// components. For a strict containment check use
    /// [`check_read_containment`](Self::check_read_containment).
    pub fn allows_read(&self, path: &Path) -> bool {
        self.read_roots.iter().any(|root| path.starts_with(root))
            || self.write_roots.iter().any(|root| path.starts_with(root))
    }

    /// Returns `true` when `path` starts with any write root.
    ///
    /// Note: this is a prefix check — for strict containment use
    /// [`check_write_containment`](Self::check_write_containment).
    pub fn allows_write(&self, path: &Path) -> bool {
        self.write_roots.iter().any(|root| path.starts_with(root))
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        let command = command.trim().to_lowercase();
        if command.is_empty() {
            return false;
        }
        !self.blocked_commands.iter().any(|entry| {
            let entry = entry.as_str().trim().to_lowercase();
            command == entry || command.starts_with(&format!("{entry} "))
        })
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.allowed_harnesses.iter().any(|entry| entry == harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.web_allowed
    }

    // ── new containment surface ──────────────────────────────────

    /// Strictly check that `path` is lexically contained within at least
    /// one read or write root.
    ///
    /// Unlike [`allows_read`](Self::allows_read) this normalises `..` and `.`
    /// components and rejects empty or escaping paths.
    pub fn check_read_containment(&self, path: &Path) -> Result<(), ContainmentError> {
        let all_roots: Vec<PathBuf> = self
            .read_roots
            .iter()
            .chain(self.write_roots.iter())
            .cloned()
            .collect();
        check_containment(&all_roots, path)
    }

    /// Strictly check that `path` is lexically contained within at least
    /// one write root.
    pub fn check_write_containment(&self, path: &Path) -> Result<(), ContainmentError> {
        check_containment(&self.write_roots, path)
    }
}

// ── ScopeGuard ───────────────────────────────────────────────────────

/// Owned guard wrapping a [`Scope`] for use by approval gates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeGuard {
    scope: Scope,
}

impl ScopeGuard {
    pub fn new(scope: Scope) -> Self {
        Self { scope }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    // ── existing delegation ──────────────────────────────────────

    pub fn allows_read(&self, path: &Path) -> bool {
        self.scope.allows_read(path)
    }

    pub fn allows_write(&self, path: &Path) -> bool {
        self.scope.allows_write(path)
    }

    pub fn command_allowed(&self, command: &str) -> bool {
        self.scope.command_allowed(command)
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.scope.harness_allowed(harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.scope.web_allowed()
    }

    // ── new containment delegation ───────────────────────────────

    pub fn check_read_containment(&self, path: &Path) -> Result<(), ContainmentError> {
        self.scope.check_read_containment(path)
    }

    pub fn check_write_containment(&self, path: &Path) -> Result<(), ContainmentError> {
        self.scope.check_write_containment(path)
    }
}

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── lexical_normalize ────────────────────────────────────────

    #[test]
    fn normalize_absolute_no_ops() {
        assert_eq!(
            lexical_normalize(Path::new("/a/b/c")),
            Some(PathBuf::from("/a/b/c"))
        );
    }

    #[test]
    fn normalize_resolves_current_dir() {
        assert_eq!(
            lexical_normalize(Path::new("/a/./b/./c")),
            Some(PathBuf::from("/a/b/c"))
        );
    }

    #[test]
    fn normalize_resolves_parent_dir() {
        assert_eq!(
            lexical_normalize(Path::new("/a/b/../c")),
            Some(PathBuf::from("/a/c"))
        );
    }

    #[test]
    fn normalize_multiple_parent_dirs() {
        assert_eq!(
            lexical_normalize(Path::new("/a/b/c/../../d")),
            Some(PathBuf::from("/a/d"))
        );
    }

    #[test]
    fn normalize_relative_path() {
        assert_eq!(
            lexical_normalize(Path::new("a/b/c")),
            Some(PathBuf::from("a/b/c"))
        );
    }

    #[test]
    fn normalize_relative_with_parent_dir() {
        assert_eq!(
            lexical_normalize(Path::new("a/b/../c")),
            Some(PathBuf::from("a/c"))
        );
    }

    #[test]
    fn normalize_escapes_root() {
        assert_eq!(lexical_normalize(Path::new("/a/../..")), None);
        assert_eq!(lexical_normalize(Path::new("/..")), None);
        assert_eq!(lexical_normalize(Path::new("/a/b/../../..")), None);
    }

    #[test]
    fn normalize_relative_starting_with_parent() {
        // "../foo" — starts with ParentDir, cannot resolve
        assert_eq!(lexical_normalize(Path::new("../foo")), None);
    }

    #[test]
    fn normalize_empty_path() {
        assert_eq!(lexical_normalize(Path::new("")), None);
    }

    #[test]
    fn normalize_only_dots() {
        // Just "." components on a relative path — resolves to nothing
        assert_eq!(lexical_normalize(Path::new("./.")), None);
    }

    // ── check_containment ────────────────────────────────────────

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/home/user/project")]
    }

    #[test]
    fn containment_ok_nested_path() {
        assert_eq!(
            check_containment(&roots(), Path::new("/home/user/project/src/main.rs")),
            Ok(())
        );
    }

    #[test]
    fn containment_ok_root_itself() {
        assert_eq!(
            check_containment(&roots(), Path::new("/home/user/project")),
            Ok(())
        );
    }

    #[test]
    fn containment_ok_with_dot_components() {
        assert_eq!(
            check_containment(&roots(), Path::new("/home/user/project/./src/./main.rs")),
            Ok(())
        );
    }

    #[test]
    fn containment_ok_with_valid_parent_dirs() {
        assert_eq!(
            check_containment(&roots(), Path::new("/home/user/project/src/../lib/util.rs")),
            Ok(())
        );
    }

    #[test]
    fn containment_rejects_empty_path() {
        assert_eq!(
            check_containment(&roots(), Path::new("")),
            Err(ContainmentError::EmptyPath)
        );
    }

    #[test]
    fn containment_rejects_escape() {
        assert_eq!(
            check_containment(&roots(), Path::new("/home/user/project/../../etc/passwd")),
            Err(ContainmentError::PathNotUnderAnyRoot {
                path: PathBuf::from("/home/etc/passwd"),
            })
        );
    }

    #[test]
    fn containment_rejects_root_escape() {
        assert_eq!(
            check_containment(&roots(), Path::new("/home/user/project/../../../..")),
            Err(ContainmentError::PathEscapesRoot {
                path: PathBuf::from("/home/user/project/../../../.."),
            })
        );
    }

    #[test]
    fn containment_rejects_relative_starting_with_parent() {
        assert_eq!(
            check_containment(&roots(), Path::new("../secret")),
            Err(ContainmentError::PathEscapesRoot {
                path: PathBuf::from("../secret"),
            })
        );
    }

    #[test]
    fn containment_rejects_path_outside_all_roots() {
        assert_eq!(
            check_containment(&roots(), Path::new("/other/project/file.rs")),
            Err(ContainmentError::PathNotUnderAnyRoot {
                path: PathBuf::from("/other/project/file.rs"),
            })
        );
    }

    #[test]
    fn containment_matches_any_root() {
        let multi_roots = vec![
            PathBuf::from("/home/user/project"),
            PathBuf::from("/opt/shared"),
        ];
        assert_eq!(
            check_containment(&multi_roots, Path::new("/opt/shared/lib/util.rs")),
            Ok(())
        );
    }

    #[test]
    fn containment_requires_normalized_root_match() {
        // Root with `..` that normalises to the actual root.
        let tricky_roots = vec![PathBuf::from("/home/user/project/sub/..")];
        // Normalised root → /home/user/project
        assert_eq!(
            check_containment(&tricky_roots, Path::new("/home/user/project/src/main.rs")),
            Ok(())
        );
    }

    // ── Scope containment methods ────────────────────────────────

    fn sample_scope() -> Scope {
        Scope::new(
            vec![PathBuf::from("/allowed/read")],
            vec![PathBuf::from("/allowed/write")],
            vec!["shell".into()],
            vec!["rm -rf".into()],
            true,
        )
    }

    #[test]
    fn scope_check_read_containment_ok() {
        let scope = sample_scope();
        assert_eq!(
            scope.check_read_containment(Path::new("/allowed/read/docs/note.md")),
            Ok(())
        );
        // Also allowed via write root
        assert_eq!(
            scope.check_read_containment(Path::new("/allowed/write/output.md")),
            Ok(())
        );
    }

    #[test]
    fn scope_check_read_containment_rejects_outside() {
        let scope = sample_scope();
        assert!(
            scope
                .check_read_containment(Path::new("/other/place/file.txt"))
                .is_err()
        );
    }

    #[test]
    fn scope_check_read_containment_rejects_escape() {
        let scope = sample_scope();
        assert!(
            scope
                .check_read_containment(Path::new("/allowed/read/../.."))
                .is_err()
        );
    }

    #[test]
    fn scope_check_write_containment_ok() {
        let scope = sample_scope();
        assert_eq!(
            scope.check_write_containment(Path::new("/allowed/write/output.md")),
            Ok(())
        );
    }

    #[test]
    fn scope_check_write_containment_rejects_read_only() {
        let scope = sample_scope();
        assert!(
            scope
                .check_write_containment(Path::new("/allowed/read/docs/note.md"))
                .is_err()
        );
    }

    // ── ScopeGuard delegation ────────────────────────────────────

    #[test]
    fn guard_delegates_containment() {
        let scope = sample_scope();
        let guard = ScopeGuard::new(scope);
        assert_eq!(
            guard.check_read_containment(Path::new("/allowed/read/docs/note.md")),
            Ok(())
        );
    }
}
