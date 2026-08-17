use super::*;
use std::{env, fs, path::PathBuf, process};

#[test]
fn scan_skips_instance_state_when_root_contains_instance() -> Result<(), Box<dyn std::error::Error>>
{
    let root = env::temp_dir().join(format!("maestria-watcher-instance-root-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("system"))?;
    fs::write(root.join("research.md"), "research")?;
    fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;

    let manifest = InstanceManifest {
        schema_version: 2,
        realm_id: maestria_test_support::realm_id(10)?,
        root: instance,
        read_roots: vec![root.clone()],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };
    let observations = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?.0;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("research.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_preserves_relative_manifest_scope() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(format!(".maestria-watcher-relative-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("note.md"), "relative note")?;

    let manifest = test_manifest(root.clone())?;
    let observations = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?.0;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_allows_read_root_nested_in_instance() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-nested-root-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    let nested = instance.join("workspace");
    fs::create_dir_all(&nested)?;
    fs::write(nested.join("note.md"), "nested note")?;

    let manifest = InstanceManifest {
        schema_version: 2,
        realm_id: maestria_test_support::realm_id(10)?,
        root: instance,
        read_roots: vec![nested],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };
    let observations = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?.0;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_excludes_instance_manifest_and_preserves_alias_scope()
-> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-instance-alias-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    let instance = root.join("instance");
    fs::create_dir_all(instance.join("system"))?;
    fs::create_dir_all(instance.join("workspace"))?;
    fs::write(instance.join("manifest.txt"), "root=/tmp/secret")?;
    fs::write(instance.join("system").join(WATCH_STATE_FILE), "{}")?;
    fs::write(instance.join("workspace").join("note.md"), "user note")?;

    let manifest = InstanceManifest {
        schema_version: 2,
        realm_id: maestria_test_support::realm_id(10)?,
        root: instance.clone(),
        read_roots: vec![instance.join(".")],
        excluded_patterns: Vec::new(),
        embeddings: None,
        ocr: None,
        visual: None,
        sparse: None,
    };
    let observations = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?.0;

    assert_eq!(observations.len(), 1);
    assert!(observations[0].path.ends_with("workspace/note.md"));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_is_deterministic_and_skips_sensitive_files() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-test-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("note.md"), "note")?;
    fs::write(root.join(".env"), "secret")?;
    let manifest = test_manifest(root.clone())?;
    let (first, signatures) = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?;
    let recorded: std::collections::BTreeMap<String, String> = first
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .collect();
    let (second, _) = scan_manifest(&manifest, &signatures, &recorded)?;
    assert_eq!(first.len(), 1);
    // Unchanged files are not re-read on the next scan (issue #440).
    assert!(
        second.is_empty(),
        "unchanged files must not be re-read, got {:?}",
        second
    );
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_respects_gitignore() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-gitignore-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("tracked.md"), "tracked content")?;
    fs::write(root.join("ignored.md"), "ignored content")?;
    fs::write(root.join(".gitignore"), "ignored.md")?;
    let manifest = test_manifest(root.clone())?;
    let observations = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?.0;
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
    let root = env::temp_dir().join(format!("maestria-watcher-change-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("stable.md"), "stable content")?;
    let manifest = test_manifest(root.clone())?;
    let (first, signatures) = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?;
    assert_eq!(first.len(), 1);
    let recorded: std::collections::BTreeMap<String, String> = first
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .collect();

    // Touch the file: the mtime changes, so the next scan must re-read it.
    let stable = root.join("stable.md");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    let file = fs::File::options().write(true).open(&stable)?;
    file.set_modified(later)?;

    let (second, second_signatures) = scan_manifest(&manifest, &signatures, &recorded)?;
    assert_eq!(second.len(), 1, "changed file must be re-read");
    assert_ne!(second_signatures, signatures);

    // No changes: nothing is re-read.
    let recorded2: std::collections::BTreeMap<String, String> = second
        .iter()
        .map(|observation| (source_key(&observation.path), observation.hash.clone()))
        .chain(
            recorded
                .iter()
                .map(|(key, hash)| (key.clone(), hash.clone())),
        )
        .collect();
    let (third, _) = scan_manifest(&manifest, &second_signatures, &recorded2)?;
    assert!(third.is_empty(), "unchanged files must not be re-read");
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn scan_respects_ignore_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::temp_dir().join(format!("maestria-watcher-ignore-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("ok.md"), "ok")?;
    fs::write(root.join("ignored.md"), "should be ignored")?;
    fs::write(root.join(".ignore"), "ignored.md")?;
    let manifest = test_manifest(root.clone())?;
    let observations = scan_manifest(&manifest, &BTreeMap::new(), &BTreeMap::new())?.0;
    assert_eq!(observations.len(), 1);
    assert!(
        observations[0].path.ends_with("ok.md"),
        "only ok.md should appear, got: {:?}",
        observations[0].path
    );
    fs::remove_dir_all(root)?;
    Ok(())
}
