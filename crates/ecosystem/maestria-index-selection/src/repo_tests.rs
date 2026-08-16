//! Repository-mode scan tests (submodule of `tests` so the shared
//! fixture helpers are reused, never copied).

use super::*;
use crate::repo::{collect_repository_files, scan_repository_candidates};

// ---------------------------------------------------------------------------
// repository-mode scan
// ---------------------------------------------------------------------------

/// A two-crate workspace fixture: code-heavy `crates`, a generated dump, a
/// mixed directory, manifests, and skipped output directories.
fn write_repository_fixture(directory: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for index in 0..50 {
        write_file(
            &directory.join(format!("crates/one/src/f{index:03}.rs")),
            "pub fn f() {}\n",
        )?;
    }
    for index in 0..300 {
        write_file(
            &directory.join(format!("dump/f{index:03}.json")),
            "{\"k\":1}",
        )?;
    }
    write_file(&directory.join("mixed/readme.md"), "# mixed")?;
    write_file(&directory.join("mixed/a.json"), "{}")?;
    write_file(&directory.join("mixed/b.json"), "{}")?;
    write_file(&directory.join("mixed/c.json"), "{}")?;
    write_file(&directory.join("mixed/d.json"), "{}")?;
    // Manifests count toward the population.
    write_file(&directory.join("Cargo.toml"), "[workspace]\n")?;
    // Skipped output and version-control paths.
    write_file(&directory.join("target/x.rs"), "fn x() {}")?;
    write_file(&directory.join("node_modules/pkg/y.js"), "y()")?;
    write_file(&directory.join("dist/bundle.js"), "bundle")?;
    write_file(&directory.join("build/out.o"), "object")?;
    write_file(&directory.join("vendor/pkg.egg-info/PKG-INFO"), "meta")?;
    write_file(&directory.join(".git/config"), "[core]\n")?;
    write_file(&directory.join(".env.local"), "TOKEN=secret")?;
    Ok(())
}

#[test]
fn repository_scan_classifies_repo_fixture_tree() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_repository_fixture(directory.path())?;

    let tree = scan_repository_candidates(directory.path())?;
    assert_eq!(tree.class, Class::Recommended);
    assert_eq!(tree.policy, IndexPolicy::everything());
    // 50 crate sources + 300 dump files + 5 mixed + 1 root manifest.
    assert_eq!(tree.file_count, 356);

    let crates = tree
        .children
        .iter()
        .find(|child| child.path.ends_with("crates"))
        .ok_or("crates child missing from candidate tree")?;
    assert_eq!(crates.class, Class::Recommended);
    assert_eq!(crates.policy, IndexPolicy::everything());
    assert_eq!(crates.file_count, 50);

    let dump = tree
        .children
        .iter()
        .find(|child| child.path.ends_with("dump"))
        .ok_or("dump child missing from candidate tree")?;
    assert_eq!(dump.class, Class::Noise);
    assert_eq!(dump.policy, IndexPolicy::filtered());
    assert_eq!(dump.file_count, 300);

    let mixed = tree
        .children
        .iter()
        .find(|child| child.path.ends_with("mixed"))
        .ok_or("mixed child missing from candidate tree")?;
    assert_eq!(mixed.class, Class::Maybe);
    assert_eq!(mixed.policy, IndexPolicy::filtered());
    assert_eq!(mixed.file_count, 5);

    // The root manifest counts toward the root population (direct files
    // form their own single-file leaf groups, mirroring the home scan).
    assert_eq!(tree.file_count, 356);
    Ok(())
}

#[test]
fn repository_collection_excludes_skipped_and_privacy_paths()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_repository_fixture(directory.path())?;

    let files = collect_repository_files(directory.path())?;
    let relative = relative_files(directory.path(), &files)?;

    assert!(
        relative.iter().all(|path| {
            !path.starts_with("target")
                && !path.starts_with("node_modules")
                && !path.starts_with("dist")
                && !path.starts_with("build")
                && !path.starts_with("vendor")
                && !path.starts_with(".git")
                && !path.starts_with(".env")
        }),
        "skipped paths leaked into the repository population: {relative:?}"
    );
    assert!(relative.contains(&PathBuf::from("Cargo.toml")));
    assert!(relative.contains(&PathBuf::from("crates/one/src/f000.rs")));
    assert!(relative.contains(&PathBuf::from("dump/f000.json")));
    Ok(())
}

#[test]
fn repository_scan_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_repository_fixture(directory.path())?;

    let first = scan_repository_candidates(directory.path())?;
    let second = scan_repository_candidates(directory.path())?;

    assert_eq!(
        serde_json::to_string(&first)?,
        serde_json::to_string(&second)?,
        "two scans of the same repository must produce identical trees"
    );
    Ok(())
}

#[test]
fn repository_scan_applies_tree_bound() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    for group in 0..20 {
        for subgroup in 0..20 {
            write_file(
                &directory
                    .path()
                    .join(format!("g{group:02}/s{subgroup:02}/f.rs")),
                "fn f() {}",
            )?;
        }
    }
    let mut tree = scan_repository_candidates(directory.path())?;
    assert_eq!(tree.children.len(), 20);
    assert!(
        tree.children.iter().all(|child| child.children.len() == 20),
        "unbounded repository tree must keep every level"
    );

    bound_candidate_tree(&mut tree);
    assert_eq!(tree.children.len(), 12);
    assert!(
        tree.children.iter().all(|child| child.children.len() == 12),
        "bound must cap children per node at 12"
    );
    assert!(
        tree.children
            .iter()
            .all(|child| child.children.iter().all(|leaf| leaf.children.is_empty())),
        "bound must drop the third level"
    );
    Ok(())
}

#[test]
fn repository_scan_missing_root_is_an_error() {
    let missing = Path::new("/definitely/not/a/real/maestria-repo-path");
    assert!(scan_repository_candidates(missing).is_err());
    assert!(collect_repository_files(missing).is_err());
}

#[test]
fn repository_collection_honors_the_repo_gitignore() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("src/lib.rs"), "pub fn real() {}\n")?;
    write_file(&directory.path().join("vendor/pkg/bundle.js"), "bundle")?;
    write_file(&directory.path().join("generated/out.json"), "{}")?;
    // The repository's own .gitignore is the per-repo standard for what is
    // not source: ignored directories never enter the population.
    write_file(
        &directory.path().join(".gitignore"),
        "vendor/\ngenerated/\n",
    )?;

    let files = collect_repository_files(directory.path())?;
    let relative = relative_files(directory.path(), &files)?;
    assert!(
        relative
            .iter()
            .all(|path| !path.starts_with("vendor") && !path.starts_with("generated")),
        "gitignored directories leaked into the population: {relative:?}"
    );
    assert_eq!(
        relative,
        vec![PathBuf::from("src/lib.rs")],
        "only non-ignored sources may be collected"
    );

    let tree = scan_repository_candidates(directory.path())?;
    assert_eq!(tree.file_count, 1);
    assert!(
        tree.children
            .iter()
            .all(|child| !child.path.ends_with("vendor")),
        "gitignored directories must not be classified"
    );
    Ok(())
}
