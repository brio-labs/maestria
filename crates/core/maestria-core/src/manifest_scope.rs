use std::path::{Component, Path, PathBuf};

pub(super) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
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
