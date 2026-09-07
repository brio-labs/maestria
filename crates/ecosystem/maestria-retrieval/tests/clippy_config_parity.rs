//! Guards the per-crate clippy configuration contract: this crate's
//! `clippy.toml` must equal the root configuration except for the single
//! sanctioned `std::time::Instant::now` omission that keeps the
//! `MonotonicInstant` abstraction constructible.

use std::path::Path;

fn disallowed_methods(path: &Path) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let mut section = false;
    let mut methods = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "disallowed-methods = [" {
            section = true;
            continue;
        }
        if section {
            if trimmed == "]" {
                section = false;
                continue;
            }
            let entry = trimmed
                .trim_end_matches(',')
                .trim()
                .trim_matches('"')
                .to_string();
            if !entry.is_empty() {
                methods.push(entry);
            }
        }
    }
    Ok(methods)
}

#[test]
fn retrieval_clippy_config_stays_a_root_subset_minus_the_clock_entry() -> Result<(), String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let repo_root = Path::new(manifest_dir)
        .ancestors()
        .nth(3)
        .ok_or("crate sits three levels under the repository root")?;
    let root = disallowed_methods(&repo_root.join("clippy.toml"))?;
    let local = disallowed_methods(&Path::new(manifest_dir).join("clippy.toml"))?;

    let clock_entry = "std::time::Instant::now";
    let expected: Vec<String> = root
        .iter()
        .filter(|entry| entry.as_str() != clock_entry)
        .cloned()
        .collect();
    assert_eq!(
        local, expected,
        "the retrieval clippy config drifted from the root config; keep both lists in sync except for the sanctioned monotonic-clock entry"
    );
    assert!(
        root.contains(&clock_entry.to_string()),
        "the root config must keep banning std::time::Instant::now outside this crate"
    );
    Ok(())
}
