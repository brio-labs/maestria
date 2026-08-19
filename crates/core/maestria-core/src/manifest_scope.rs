use std::path::Path;

pub use maestria_governance::lexical_normalize;

pub fn path_matches_pattern(path: &Path, pattern: &str) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy();
        pattern == name
            || (pattern == ".env.*" && name.starts_with(".env."))
            || (pattern == "*.pem" && name.ends_with(".pem"))
            || (pattern == "*.key" && name.ends_with(".key"))
    })
}
