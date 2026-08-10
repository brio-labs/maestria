//! Behavior-defining unit tests for the choice layer.

use crate::candidates::scan_candidates;
use crate::classify::{classify, default_policy, Class};
use crate::policy::{group_by_child, select_source, IndexPolicy, Selection};
use crate::profile::{load_profile, save_profile, IndexSelectionProfile};
use crate::scan::{
    collect_files, dir_features, is_privacy_excluded_path, is_supported_source_file,
};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    fn create() -> Result<Self, Box<dyn std::error::Error>> {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-index-selection-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn write_file(path: &Path, contents: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

fn write_file_bytes(path: &Path, contents: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(contents)?;
    Ok(())
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

fn symlink_unavailable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Unsupported
    )
}

fn features(
    file_count: usize,
    total_bytes: u64,
    mean_bytes: u64,
    doc_share: f64,
    code_share: f64,
    single_ext_share: f64,
    minified_share: f64,
) -> crate::scan::DirFeatures {
    crate::scan::DirFeatures {
        file_count,
        total_bytes,
        max_file_bytes: 0,
        mean_bytes,
        doc_share,
        code_share,
        single_ext_share,
        minified_share,
    }
}

// ---------------------------------------------------------------------------
// classify
// ---------------------------------------------------------------------------

#[test]
fn generated_dump_is_noise() {
    let dump = features(300, 300 * 1024, 1024, 0.0, 0.0, 0.95, 0.0);
    assert_eq!(
        classify(&dump, false, Path::new("/tmp/root/dump")),
        Class::Noise
    );
}

#[test]
fn home_noise_components_only_apply_when_scanning_home() {
    let features = features(10, 10 * 1024, 1024, 0.9, 0.0, 0.5, 0.0);
    let config = Path::new("/home/user/.config/foo");
    assert_eq!(classify(&features, true, config), Class::Noise);
    assert_eq!(classify(&features, false, config), Class::Recommended);
    // Non-noise components are not name-excluded even under home_root.
    assert_eq!(
        classify(&features, true, Path::new("/home/user/Notes")),
        Class::Recommended
    );
}

#[test]
fn doc_directory_is_recommended() {
    let docs = features(100, 500 * 1024, 5 * 1024, 0.8, 0.0, 0.6, 0.0);
    assert_eq!(
        classify(&docs, false, Path::new("/tmp/root/docs")),
        Class::Recommended
    );
}

#[test]
fn code_directory_is_recommended() {
    let code = features(800, 8 * 1024 * 1024, 10 * 1024, 0.0, 0.5, 0.4, 0.0);
    assert_eq!(
        classify(&code, false, Path::new("/tmp/root/code")),
        Class::Recommended
    );
}

#[test]
fn mixed_small_directory_is_maybe() {
    let mixed = features(12, 60 * 1024, 5 * 1024, 0.3, 0.1, 0.3, 0.0);
    assert_eq!(
        classify(&mixed, false, Path::new("/tmp/root/mixed")),
        Class::Maybe
    );
}

#[test]
fn minified_heavy_directory_is_noise() {
    let bundles = features(40, 20 * 1024 * 1024, 512 * 1024, 0.0, 0.0, 0.8, 0.8);
    assert_eq!(
        classify(&bundles, false, Path::new("/tmp/root/vendor-js")),
        Class::Noise
    );
}

// ---------------------------------------------------------------------------
// default_policy
// ---------------------------------------------------------------------------

#[test]
fn default_policy_maps_classes() {
    assert_eq!(
        default_policy(Class::Recommended),
        IndexPolicy::everything()
    );
    assert_eq!(default_policy(Class::Maybe), IndexPolicy::filtered());
    assert_eq!(default_policy(Class::Noise), IndexPolicy::filtered());
}

// ---------------------------------------------------------------------------
// IndexPolicy::display
// ---------------------------------------------------------------------------

#[test]
fn display_exact_strings() {
    assert_eq!(IndexPolicy::everything().display(), "index everything");
    assert_eq!(
        IndexPolicy::filtered().display(),
        "skip >1MiB, generated dumps, minified bundles"
    );
    assert_eq!(
        IndexPolicy {
            max_file_bytes: 0,
            skip_generated: false,
            skip_minified: true,
        }
        .display(),
        "minified bundles"
    );
}

#[test]
fn is_filtered_requires_any_switch() {
    assert!(!IndexPolicy::everything().is_filtered());
    assert!(IndexPolicy::filtered().is_filtered());
    assert!(!IndexPolicy {
        max_file_bytes: 0,
        skip_generated: false,
        skip_minified: false,
    }
    .is_filtered());
}

// ---------------------------------------------------------------------------
// select_source
// ---------------------------------------------------------------------------

#[test]
fn large_files_skipped_only_when_limit_set() {
    let policy = IndexPolicy {
        max_file_bytes: 1024 * 1024,
        skip_generated: false,
        skip_minified: false,
    };
    assert!(matches!(
        select_source(Path::new("/tmp/x.md"), 1024 * 1024, policy),
        Selection::Index
    ));
    assert!(matches!(
        select_source(Path::new("/tmp/x.md"), 1024 * 1024 + 1, policy),
        Selection::Skip("large")
    ));
    // No limit set: everything passes.
    assert!(matches!(
        select_source(Path::new("/tmp/x.md"), 1 << 30, IndexPolicy::everything()),
        Selection::Index
    ));
}

#[test]
fn minified_single_line_bundle_skipped_when_switch_on() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let single_line = directory.path().join("bundle.js");
    let mut contents = Vec::with_capacity(512 * 1024);
    contents.extend_from_slice(b"var a=1;");
    contents.resize(512 * 1024, b' ');
    write_file_bytes(&single_line, &contents)?;

    let multi_line = directory.path().join("lines.js");
    let mut lines = Vec::with_capacity(512 * 1024);
    for index in 0..(512 * 1024 / 20) {
        lines.extend_from_slice(format!("console.log({index});\n").as_bytes());
    }
    write_file_bytes(&multi_line, &lines)?;

    let policy = IndexPolicy {
        max_file_bytes: 0,
        skip_generated: false,
        skip_minified: true,
    };
    assert!(matches!(
        select_source(&single_line, 512 * 1024, policy),
        Selection::Skip("minified")
    ));
    assert!(matches!(
        select_source(&multi_line, 512 * 1024, policy),
        Selection::Index
    ));
    // Below the 256 KiB floor a single-line file is not called minified.
    let small = directory.path().join("small.js");
    write_file(&small, "var a=1;")?;
    assert!(matches!(select_source(&small, 9, policy), Selection::Index));
    Ok(())
}

// ---------------------------------------------------------------------------
// group_by_child
// ---------------------------------------------------------------------------

#[test]
fn group_by_child_aggregates_counts_and_sizes() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("Dev/a.md"), "a")?;
    write_file(&directory.path().join("Dev/b.md"), "bb")?;
    write_file(&directory.path().join("Notes/c.md"), "ccc")?;
    let files = collect_files(directory.path(), true)?;
    let groups = group_by_child(directory.path(), &files);
    assert_eq!(groups.len(), 2);
    let dev = groups.iter().find(|(name, _, _)| name.ends_with("Dev"));
    assert!(dev.is_some_and(|(_, count, bytes)| *count == 2 && *bytes == 3));
    let notes = groups.iter().find(|(name, _, _)| name.ends_with("Notes"));
    assert!(notes.is_some_and(|(_, count, bytes)| *count == 1 && *bytes == 3));
    Ok(())
}

// ---------------------------------------------------------------------------
// dir_features
// ---------------------------------------------------------------------------

#[test]
fn dir_features_aggregates_counts_and_shares() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("docs/a.md"), "aaa")?;
    write_file(&directory.path().join("docs/b.md"), "bb")?;
    write_file(&directory.path().join("src/c.rs"), "cccc")?;
    write_file(&directory.path().join("src/d.json"), "dddddd")?;
    let files = collect_files(directory.path(), true)?;
    let features = dir_features(directory.path(), &files);
    assert_eq!(features.file_count, 4);
    assert_eq!(features.total_bytes, 3 + 2 + 4 + 6);
    assert_eq!(features.mean_bytes, 3);
    assert_eq!(features.doc_share, 0.5);
    assert_eq!(features.code_share, 0.25);
    assert_eq!(features.single_ext_share, 0.5);
    assert_eq!(features.max_file_bytes, 6);
    Ok(())
}

// ---------------------------------------------------------------------------
// scan_candidates
// ---------------------------------------------------------------------------

#[test]
fn scan_candidates_classifies_fixture_tree() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("docs/a.md"), "# doc")?;
    write_file(&directory.path().join("code/main.rs"), "fn main() {}")?;
    for index in 0..300 {
        write_file(
            &directory.path().join(format!("dump/f{index}.json")),
            "{\"k\":1}",
        )?;
    }
    let tree = scan_candidates(directory.path())?;
    assert_eq!(tree.class, Class::Recommended);
    assert_eq!(tree.policy, IndexPolicy::everything());
    assert_eq!(tree.file_count, 302);

    let docs = tree
        .children
        .iter()
        .find(|child| child.path.ends_with("docs"))
        .ok_or("docs child missing from candidate tree")?;
    assert_eq!(docs.class, Class::Recommended);
    assert_eq!(docs.policy, IndexPolicy::everything());
    assert_eq!(docs.file_count, 1);

    let code = tree
        .children
        .iter()
        .find(|child| child.path.ends_with("code"))
        .ok_or("code child missing from candidate tree")?;
    assert_eq!(code.class, Class::Recommended);
    assert_eq!(code.file_count, 1);

    let dump = tree
        .children
        .iter()
        .find(|child| child.path.ends_with("dump"))
        .ok_or("dump child missing from candidate tree")?;
    assert_eq!(dump.class, Class::Noise);
    assert_eq!(dump.policy, IndexPolicy::filtered());
    assert_eq!(dump.file_count, 300);
    assert_eq!(dump.total_bytes, 300 * 7);
    // Each direct file is its own single-file leaf group; the dump node's
    // own decision covers the whole subtree, so the leaves are never
    // prompted individually.
    assert_eq!(dump.children.len(), 300);
    assert!(dump
        .children
        .iter()
        .all(|leaf| leaf.children.is_empty() && leaf.file_count == 1));
    Ok(())
}

// ---------------------------------------------------------------------------
// profile round-trip
// ---------------------------------------------------------------------------

#[test]
fn profile_save_load_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let profile_path = directory.path().join("index-selection.json");
    assert_eq!(load_profile(&profile_path)?, None);

    let profile = IndexSelectionProfile {
        root: PathBuf::from("/tmp/choice"),
        includes: vec![
            PathBuf::from("/tmp/choice/docs"),
            PathBuf::from("/tmp/choice/code"),
        ],
        policies: [
            (PathBuf::from("/tmp/choice/docs"), IndexPolicy::filtered()),
            (PathBuf::from("/tmp/choice/code"), IndexPolicy::everything()),
        ]
        .into_iter()
        .collect(),
    };
    save_profile(&profile_path, &profile)?;
    assert_eq!(load_profile(&profile_path)?, Some(profile));

    // Malformed content propagates as an error.
    write_file(&profile_path, "{not json")?;
    assert!(load_profile(&profile_path).is_err());
    Ok(())
}

// ---------------------------------------------------------------------------
// Ported collection assertions (from the CLI's collection tests)
// ---------------------------------------------------------------------------

#[test]
fn exclusion_policy_covers_sensitive_and_build_paths() {
    for path in [
        "workspace/.env",
        "workspace/.env.local",
        "workspace/cert.pem",
        "workspace/deploy.key",
        "workspace/secrets/token.md",
        "workspace/.ssh/config",
        "workspace/.gnupg/pubring.kbx",
        "workspace/node_modules/package/index.js",
        "workspace/target/debug/app",
        "workspace/dist/bundle.js",
        "workspace/build/output.o",
    ] {
        assert!(
            is_privacy_excluded_path(Path::new(path)),
            "expected {path} to be excluded from indexing"
        );
    }

    for path in [
        "workspace/notes/readme.md",
        "workspace/src/building.md",
        "workspace/src/targeted.md",
    ] {
        assert!(
            !is_privacy_excluded_path(Path::new(path)),
            "expected {path} to be indexable"
        );
    }
}

#[test]
fn collecting_single_env_file_is_rejected_by_privacy_policy(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let env_file = directory.path().join(".env");
    write_file(&env_file, "TOKEN=secret")?;

    let error = match collect_files(&env_file, false) {
        Err(e) => e,
        Ok(_) => return Err("single .env files must not be accepted for indexing".into()),
    };

    assert!(
        error.to_string().contains("privacy policy"),
        "unexpected error for excluded .env file: {error}"
    );
    Ok(())
}

#[test]
fn collecting_single_unsupported_file_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let unsupported_file = directory.path().join("notes.sqlite");
    write_file(&unsupported_file, "not text evidence")?;

    let error = match collect_files(&unsupported_file, false) {
        Err(e) => e,
        Ok(_) => return Err("single unsupported files must not be accepted for indexing".into()),
    };

    assert!(
        error.to_string().contains("unsupported index file type"),
        "unexpected error for unsupported file: {error}"
    );
    Ok(())
}

#[test]
fn pdf_is_supported_index_path() {
    assert!(is_supported_source_file(Path::new("paper.pdf")));
    assert!(is_supported_source_file(Path::new("paper.PDF")));
    assert!(is_supported_source_file(Path::new("docs/report.Pdf")));
}

#[test]
fn collecting_single_pdf_is_accepted() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let pdf_file = directory.path().join("paper.pdf");
    write_file(&pdf_file, "minimal pdf bytes")?;

    let files = collect_files(&pdf_file, false)?;

    assert_eq!(files, vec![pdf_file]);
    Ok(())
}

#[test]
fn recursive_collection_includes_pdf_files() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(
        &directory.path().join("docs/report.pdf"),
        "minimal pdf bytes",
    )?;
    write_file(
        &directory.path().join("docs/cache.sqlite"),
        "opaque database",
    )?;

    let files = collect_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("docs/report.pdf"), PathBuf::from("note.md"),]
    );
    Ok(())
}

#[test]
fn collecting_single_symlink_is_rejected_and_recursive_collection_skips_it(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let sensitive_target = directory.path().join(".env");
    let benign_link = directory.path().join("public.md");
    let supported_note = directory.path().join("note.md");
    write_file(&sensitive_target, "TOKEN=secret")?;
    write_file(&supported_note, "# Public note")?;

    match symlink_file(&sensitive_target, &benign_link) {
        Ok(()) => {}
        Err(error) if symlink_unavailable(&error) => return Ok(()),
        Err(error) => {
            return Err(format!(
                "create symlink {} -> {}: {error}",
                benign_link.display(),
                sensitive_target.display()
            )
            .into());
        }
    }

    let error = match collect_files(&benign_link, false) {
        Err(e) => e,
        Ok(_) => return Err("single symlink files must not be accepted for indexing".into()),
    };
    assert!(
        error.to_string().contains("symlink"),
        "unexpected error for symlink file: {error}"
    );

    let files = collect_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("note.md")]
    );
    Ok(())
}

#[test]
fn collecting_path_with_symlinked_parent_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let outside = TestDirectory::create()?;
    let link = directory.path().join("linked");
    let linked_note = link.join("note.md");
    write_file(&outside.path().join("note.md"), "# Outside note")?;

    match symlink_dir(outside.path(), &link) {
        Ok(()) => {}
        Err(error) if symlink_unavailable(&error) => return Ok(()),
        Err(error) => {
            return Err(format!(
                "create directory symlink {} -> {}: {error}",
                link.display(),
                outside.path().display()
            )
            .into());
        }
    }

    let error = match collect_files(&linked_note, false) {
        Err(e) => e,
        Ok(_) => return Err("paths through symlinked parents must not be indexed".into()),
    };

    assert!(
        error.to_string().contains("symlink"),
        "unexpected symlink-parent error: {error}"
    );
    Ok(())
}

#[test]
fn recursive_collection_skips_unsupported_files_and_keeps_supported_markdown(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(
        &directory.path().join("docs/guide.markdown"),
        "# Normal guide",
    )?;
    write_file(
        &directory.path().join("docs/cache.sqlite"),
        "opaque database",
    )?;
    write_file(&directory.path().join("image.png"), "not text evidence")?;

    let files = collect_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![
            PathBuf::from("docs/guide.markdown"),
            PathBuf::from("note.md"),
        ]
    );
    Ok(())
}

#[test]
fn recursive_collection_skips_excluded_entries_and_keeps_markdown(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(&directory.path().join("docs/guide.md"), "# Normal guide")?;
    write_file(&directory.path().join(".env.local"), "TOKEN=secret")?;
    write_file(&directory.path().join("cert.pem"), "private key")?;
    write_file(&directory.path().join("deploy.key"), "private key")?;
    write_file(&directory.path().join("secrets/passwords.md"), "secret")?;
    write_file(&directory.path().join(".ssh/config"), "Host secret")?;
    write_file(&directory.path().join(".gnupg/pubring.kbx"), "keyring")?;
    write_file(
        &directory.path().join("node_modules/package/index.js"),
        "module",
    )?;
    write_file(&directory.path().join("target/debug/app"), "binary")?;
    write_file(&directory.path().join("dist/bundle.js"), "bundle")?;
    write_file(&directory.path().join("build/output.o"), "object")?;

    let files = collect_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("docs/guide.md"), PathBuf::from("note.md")]
    );
    Ok(())
}

#[test]
fn recursive_collection_respects_gitignore() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(&directory.path().join("ignored_file.md"), "ignored content")?;
    write_file(&directory.path().join(".gitignore"), "ignored_file.md")?;

    let files = collect_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("note.md")]
    );
    Ok(())
}

#[test]
fn recursive_collection_propagates_ignore_file_errors() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(&directory.path().join(".gitignore"), "{malformed")?;

    let error = match collect_files(directory.path(), true) {
        Err(e) => e,
        Ok(_) => return Err("malformed ignore files must fail traversal".into()),
    };

    assert!(
        error.to_string().contains("traversal failed"),
        "unexpected traversal error: {error}"
    );
    Ok(())
}

#[test]
fn recursive_collection_skips_hidden_files_and_directories(
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(&directory.path().join(".hidden_file.md"), "hidden")?;
    write_file(
        &directory.path().join(".hidden_dir/file.md"),
        "hidden inside dir",
    )?;

    let files = collect_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("note.md")]
    );
    Ok(())
}

fn relative_files(
    root: &Path,
    files: &[PathBuf],
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut relative = Vec::with_capacity(files.len());
    for path in files {
        relative.push(path.strip_prefix(root)?.to_path_buf());
    }
    Ok(relative)
}
