use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::helpers;

static NEXT_TEST_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

pub(super) struct TestDirectory {
    path: PathBuf,
}

impl TestDirectory {
    pub(super) fn create() -> Result<Self, Box<dyn std::error::Error>> {
        let base = std::env::temp_dir();
        for _ in 0..1000 {
            let id = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "maestria-cli-index-test-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(format!("create unique test directory under {}", base.display()).into())
    }

    pub(super) fn path(&self) -> &Path {
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

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

fn symlink_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
    )
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

#[test]
fn index_exclusion_policy_covers_sensitive_and_build_paths()
-> Result<(), Box<dyn std::error::Error>> {
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
            helpers::is_excluded_index_path(Path::new(path)),
            "expected {path} to be excluded from indexing"
        );
    }

    for path in [
        "workspace/notes/readme.md",
        "workspace/src/building.md",
        "workspace/src/targeted.md",
    ] {
        assert!(
            !helpers::is_excluded_index_path(Path::new(path)),
            "expected {path} to be indexable"
        );
    }
    Ok(())
}

#[test]
fn collecting_single_env_file_is_rejected_by_privacy_policy()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let env_file = directory.path().join(".env");
    write_file(&env_file, "TOKEN=secret")?;

    let error = match helpers::collect_index_files(&env_file, false) {
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

    let error = match helpers::collect_index_files(&unsupported_file, false) {
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
fn pdf_is_supported_index_path() -> Result<(), Box<dyn std::error::Error>> {
    assert!(helpers::is_supported_index_path(Path::new("paper.pdf")));
    assert!(helpers::is_supported_index_path(Path::new("paper.PDF")));
    assert!(helpers::is_supported_index_path(Path::new(
        "docs/report.Pdf"
    )));
    Ok(())
}

#[test]
fn collecting_single_pdf_is_accepted() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let pdf_file = directory.path().join("paper.pdf");
    write_file(&pdf_file, "minimal pdf bytes")?;

    let files = helpers::collect_index_files(&pdf_file, false)?;

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

    let files = helpers::collect_index_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("docs/report.pdf"), PathBuf::from("note.md"),]
    );
    Ok(())
}

#[test]
fn collecting_single_symlink_is_rejected_and_recursive_collection_skips_it()
-> Result<(), Box<dyn std::error::Error>> {
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

    let error = match helpers::collect_index_files(&benign_link, false) {
        Err(e) => e,
        Ok(_) => return Err("single symlink files must not be accepted for indexing".into()),
    };
    assert!(
        error.to_string().contains("symlink"),
        "unexpected error for symlink file: {error}"
    );

    let files = helpers::collect_index_files(directory.path(), true)?;

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

    let error = match helpers::collect_index_files(&linked_note, false) {
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
fn recursive_collection_skips_unsupported_files_and_keeps_supported_markdown()
-> Result<(), Box<dyn std::error::Error>> {
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

    let files = helpers::collect_index_files(directory.path(), true)?;

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
fn recursive_collection_skips_excluded_entries_and_keeps_markdown()
-> Result<(), Box<dyn std::error::Error>> {
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

    let files = helpers::collect_index_files(directory.path(), true)?;
    let relative_files = relative_files(directory.path(), &files)?;

    assert_eq!(
        relative_files,
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

    let files = helpers::collect_index_files(directory.path(), true)?;

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

    let error = match helpers::collect_index_files(directory.path(), true) {
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
fn recursive_collection_skips_hidden_files_and_directories()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    write_file(&directory.path().join("note.md"), "# Normal note")?;
    write_file(&directory.path().join(".hidden_file.md"), "hidden")?;
    write_file(
        &directory.path().join(".hidden_dir/file.md"),
        "hidden inside dir",
    )?;

    let files = helpers::collect_index_files(directory.path(), true)?;

    assert_eq!(
        relative_files(directory.path(), &files)?,
        vec![PathBuf::from("note.md")]
    );
    Ok(())
}
