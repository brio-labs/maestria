use super::*;
use std::{env, fs, path::PathBuf, process};

fn scan_with_default_budget(
    manifest: &InstanceManifest,
    previous: &BTreeMap<String, super::super::watcher_state::FileSignature>,
    recorded: &BTreeMap<String, String>,
) -> anyhow::Result<super::super::watcher_scan::ScanResult> {
    scan_manifest(
        manifest,
        previous,
        recorded,
        MAX_ADMITTED_SOURCE_BYTES,
    )
}

#[test]
fn scan_skips_instance_state_when_root_contains_instance() -> Result<(), Box<dyn std::error::Error>>
{
    let root = env::temp_dir().join(format!("sillage-watcher-instance-root-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("system"))?;
    fs::write(root.join("research.md"), "research")?;
    fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;

    let manifest = InstanceManifest {
        schema_version: 2,
        realm_id: sillage_test_support::realm_id(10)?,
        root: instance,
        read_roots: vec![root.clone()],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };
    let observations =
        scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?.observations;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("research.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_preserves_relative_manifest_scope() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(format!(".sillage-watcher-relative-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("note.md"), "relative note")?;

    let manifest = test_manifest(root.clone())?;
    let observations =
        scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?.observations;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_allows_read_root_nested_in_instance() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("sillage-watcher-nested-root-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    let nested = instance.join("workspace");
    fs::create_dir_all(&nested)?;
    fs::write(nested.join("note.md"), "nested note")?;

    let manifest = InstanceManifest {
        schema_version: 2,
        realm_id: sillage_test_support::realm_id(10)?,
        root: instance,
        read_roots: vec![nested],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };
    let observations =
        scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?.observations;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_excludes_instance_manifest_and_preserves_alias_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("sillage-watcher-instance-alias-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("system"))?;
    fs::create_dir_all(instance.join("workspace"))?;
    fs::write(instance.join("manifest.txt"), "root=/tmp/secret")?;
    fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;
    fs::write(instance.join("workspace").join("note.md"), "user note")?;

    let manifest = InstanceManifest {
        schema_version: 2,
        realm_id: sillage_test_support::realm_id(10)?,
        root: instance.clone(),
        read_roots: vec![instance.join(".")],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };
    let observations =
        scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?.observations;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("workspace/note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_is_deterministic_and_skips_sensitive_files() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("sillage-watcher-test-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("note.md"), "note")?;
    fs::write(root.join(".env"), "secret")?;
    let manifest = test_manifest(root.clone())?;
    let first = scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?;
    let recorded: std::collections::BTreeMap<String, String> = first
        .observations
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .collect();
    let second = scan_with_default_budget(&manifest, &first.signatures, &recorded)?;
    assert_eq!(first.observations.len(), 1);
    // Unchanged files are not re-read on the next scan (issue #440).
    assert!(
        second.observations.is_empty(),
        "unchanged files must not be re-read, got {:?}",
        second.observations
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_respects_gitignore() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("sillage-watcher-gitignore-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("tracked.md"), "tracked content")?;
    fs::write(root.join("ignored.md"), "ignored content")?;
    fs::write(root.join(".gitignore"), "ignored.md")?;
    let manifest = test_manifest(root.clone())?;
    let observations =
        scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?.observations;
    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].path.ends_with("tracked.md"),
        "only tracked.md should appear, got: {:?}",
        observations[0].path
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_re_reads_only_changed_files() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("sillage-watcher-change-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("stable.md"), "stable content")?;
    let manifest = test_manifest(root.clone())?;
    let first = scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?;
    assert_eq!(first.observations.len(), 1);
    let recorded: std::collections::BTreeMap<String, String> = first
        .observations
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .collect();

    // Touch the file: the mtime changes, so the next scan must re-read it.
    let stable = root.join("stable.md");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = fs::File::options().write(true).open(&stable)?;
    file.set_modified(later)?;

    let second = scan_with_default_budget(&manifest, &first.signatures, &recorded)?;
    assert_eq!(second.observations.len(), 1, "changed file must be re-read");
    assert_ne!(second.signatures, first.signatures);

    // No changes: nothing is re-read.
    let recorded2: std::collections::BTreeMap<String, String> = second
        .observations
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .chain(
            recorded
                .iter()
                .map(|(key, hash)| (key.clone(), hash.clone())),
        )
        .collect();
    let third = scan_with_default_budget(&manifest, &second.signatures, &recorded2)?;
    assert!(
        third.observations.is_empty(),
        "unchanged files must not be re-read"
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_respects_ignore_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("sillage-watcher-ignore-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("ok.md"), "ok")?;
    fs::write(root.join("ignored.md"), "should be ignored")?;
    fs::write(root.join(".ignore"), "ignored.md")?;
    let manifest = test_manifest(root.clone())?;
    let observations =
        scan_with_default_budget(&manifest, &BTreeMap::new(), &BTreeMap::new())?.observations;
    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].path.ends_with("ok.md"),
        "only ok.md should appear, got: {:?}",
        observations[0].path
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_defers_sources_until_the_admission_budget_frees() -> Result<(), Box<dyn std::error::Error>>
{
    let root = env::temp_dir().join(format!("sillage-watcher-admission-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("a.md"), "tiny")?;
    fs::write(root.join("b.md"), "deferred")?;
    let manifest = test_manifest(root.clone())?;

    let first = scan_manifest(
        &manifest,
        &BTreeMap::new(),
        &BTreeMap::new(),
        4,
    )?;
    assert_eq!(first.observations.len(), 1);
    assert_eq!(first.observations[0].bytes.len(), 4);
    assert_eq!(first.admission_deferred_files, 1);
    assert_eq!(first.oversized_files, 0);

    let recorded: BTreeMap<String, String> = first
        .observations
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .collect();
    let second = scan_manifest(
        &manifest,
        &first.signatures,
        &recorded,
        MAX_ADMITTED_SOURCE_BYTES,
    )?;
    assert_eq!(second.observations.len(), 1);
    assert!(second.observations[0].path.ends_with("b.md"));
    assert_eq!(second.admission_deferred_files, 0);
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_reports_oversized_sources_without_reading_them() -> Result<(), Box<dyn std::error::Error>>
{
    let root = env::temp_dir().join(format!(
        "sillage-watcher-oversized-admission-{}",
        process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("small.md"), "x")?;
    let oversized = root.join("oversized.md");
    fs::File::create(&oversized)?
        .set_len(u64::try_from(MAX_ADMITTED_SOURCE_BYTES)? + 1)?;
    let manifest = test_manifest(root.clone())?;

    let scan = scan_manifest(
        &manifest,
        &BTreeMap::new(),
        &BTreeMap::new(),
        MAX_ADMITTED_SOURCE_BYTES,
    )?;
    assert_eq!(scan.oversized_files, 1);
    assert_eq!(scan.admission_deferred_files, 1);
    assert_eq!(scan.observations.len(), 1);
    assert!(scan.observations[0].path.ends_with("small.md"));
    assert!(!scan.signatures.contains_key(&source_key(&oversized)));
    fs::remove_dir_all(root)?;
    Ok(())
}
