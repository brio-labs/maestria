use super::common::{TempDir, assert_index_ok, assert_init_ok, assert_ok, run, write_file};

fn parse_key_values(line: &str) -> std::collections::BTreeMap<&str, &str> {
    line.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .collect()
}

#[test]
fn exact_search_returns_immutable_evidence_and_no_match_is_explicit()
-> Result<(), Box<dyn std::error::Error>> {
    let workspace = TempDir::new("maestria-release-search-workspace")?;
    let instance = TempDir::new("maestria-release-search-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let workspace_path = workspace.path().to_string_lossy().into_owned();
    assert_init_ok(&instance_path, &workspace_path)?;
    write_file(
        workspace.path(),
        "notes.md",
        "The immutable ledger records provenance for every research claim.",
    )?;
    assert_index_ok(
        &instance_path,
        &workspace.path().join("notes.md").to_string_lossy(),
    )?;

    let output = assert_ok(&["search", "-i", &instance_path, "immutable"])?;
    let line = output
        .lines()
        .find(|line| line.starts_with("rank="))
        .ok_or("ranked evidence output missing")?;
    let fields = parse_key_values(line);
    assert!(fields.contains_key("rank"));
    assert!(fields.contains_key("artifact"));
    assert!(fields.contains_key("evidence"));
    assert!(fields.contains_key("source"));
    assert!(fields.contains_key("snippet"));
    assert!(line.contains("source=file"));
    assert!(line.contains("snippet="));

    let explained = assert_ok(&["search", "explain", "-i", &instance_path, "immutable"])?;
    assert!(
        explained.contains("strategy: \"hierarchy+graph\""),
        "hierarchy expansion missing from trace: {explained}"
    );
    let no_match = assert_ok(&["search", "-i", &instance_path, "nonexistent-token"])?;
    assert!(no_match.contains("search_status=NoEvidenceFound"));
    Ok(())
}

#[test]
fn unsupported_capabilities_are_typed_errors() -> Result<(), Box<dyn std::error::Error>> {
    let instance = TempDir::new("maestria-release-unsupported-instance")?;
    let instance_path = instance.path().to_string_lossy().into_owned();
    let root = instance.path().to_string_lossy().into_owned();
    assert_init_ok(&instance_path, &root)?;

    for (query, intent) in [
        ("repository rust function", "RepositoryCode"),
        ("visual document chart", "VisualDocument"),
        ("latest web news", "CurrentWeb"),
    ] {
        let (code, stdout, stderr) = run(&["search", "-i", &instance_path, query])?;
        assert_ne!(code, 0, "unsupported query unexpectedly succeeded: {query}");
        assert!(
            stdout.is_empty(),
            "unsupported query wrote stdout: {stdout}"
        );
        assert!(
            stderr.contains(&format!("unsupported search intent: {intent}")),
            "missing typed unsupported result for {intent}: {stderr}"
        );
    }
    Ok(())
}
