use std::path::{Component, Path, PathBuf};

/// Lexically normalize `path`, resolving `.` and `..` components.
///
/// Returns `None` when a `..` component would escape above the path root
/// (e.g. `../secret` relative to a root of `workspace`): such a path is
/// outside every lexical scope and must be denied, never collapsed.
pub(super) fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Some(normalized)
}

pub(super) fn path_matches_pattern(path: &Path, pattern: &str) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        pattern == name
            || (pattern == ".env.*" && name.starts_with(".env."))
            || (pattern == "*.pem" && name.ends_with(".pem"))
            || (pattern == "*.key" && name.ends_with(".key"))
    })
}
