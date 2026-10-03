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
/// would escape the filesystem root or the path is empty. This is the single
/// canonical implementation; consumers must not re-implement path
/// normalization with different (silent-pop) semantics.
pub fn lexical_normalize(path: &Path) -> Option<PathBuf> {
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
    read_roots: std::sync::Arc<[PathBuf]>,
    write_roots: std::sync::Arc<[PathBuf]>,
    allowed_harnesses: std::sync::Arc<[String]>,
    blocked_commands: std::sync::Arc<[String]>,
    blocked_patterns: std::sync::Arc<[String]>,
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
            read_roots: read_roots.into(),
            write_roots: write_roots.into(),
            allowed_harnesses: allowed_harnesses.into(),
            // Pre-normalize so `command_allowed` never re-trims/lowercases
            // per entry.
            blocked_commands: blocked_commands
                .into_iter()
                .map(|entry| entry.trim().to_lowercase())
                .collect(),
            blocked_patterns: Vec::new().into(),
            web_allowed,
        }
    }

    // ── existing public surface ──────────────────────────────────

    pub fn command_allowed(&self, command: &str) -> bool {
        let command = command.trim().to_lowercase();
        if command.is_empty() {
            return false;
        }
        // Entries are pre-normalized (trimmed, lowercased) at construction,
        // so the comparison allocates only the caller's lowercase command.
        !self.blocked_commands.iter().any(|entry| {
            command == *entry
                || (command.len() > entry.len()
                    && command.as_bytes().get(entry.len()) == Some(&b' ')
                    && command.starts_with(entry))
        })
    }

    pub fn harness_allowed(&self, harness: &str) -> bool {
        self.allowed_harnesses.iter().any(|entry| entry == harness)
    }

    pub fn web_allowed(&self) -> bool {
        self.web_allowed
    }

    /// Returns the read roots suitable for harness adapter path validation.
    pub fn readable_roots(&self) -> &[PathBuf] {
        &self.read_roots
    }

    pub fn blocked_patterns(&self) -> &[String] {
        &self.blocked_patterns
    }

    pub fn with_blocked_patterns(mut self, patterns: Vec<String>) -> Self {
        self.blocked_patterns = patterns.into();
        self
    }

    /// Strictly check that `path` is lexically contained within at least
    /// one read or write root.
    ///
    /// This normalises `..` and `.` components and rejects empty or escaping
    /// paths. Read roots are checked first without building a combined Vec.
    pub fn check_read_containment(&self, path: &Path) -> Result<(), ContainmentError> {
        match check_containment(&self.read_roots, path) {
            Ok(()) => Ok(()),
            Err(ContainmentError::PathNotUnderAnyRoot { .. }) => {
                check_containment(&self.write_roots, path)
            }
            Err(other) => Err(other),
        }
    }

    /// Strictly check that `path` is lexically contained within at least
    /// one write root.
    pub fn check_write_containment(&self, path: &Path) -> Result<(), ContainmentError> {
        check_containment(&self.write_roots, path)
    }
}

// ── tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod scope_tests;
