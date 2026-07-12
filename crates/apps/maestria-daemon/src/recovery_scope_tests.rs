use super::*;
use maestria_domain::{BlobId, ParserStarted};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SCOPE_TEST_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn create() -> Self {
        let id = NEXT_SCOPE_TEST_ID.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("maestria-scope-test-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_manifest(dir: &Path, read_roots: &[&str]) -> PathBuf {
    let mut lines = vec![
        "schema_version=1".to_string(),
        format!("root={}", dir.display()),
    ];
    for root in read_roots {
        lines.push(format!("read_root={root}"));
    }
    lines.push("excluded_pattern=.env".to_string());
    lines.push("excluded_pattern=.env.*".to_string());
    lines.push("excluded_pattern=*.pem".to_string());
    lines.push("excluded_pattern=*.key".to_string());
    lines.push("excluded_pattern=.ssh".to_string());
    lines.push("excluded_pattern=.gnupg".to_string());
    lines.push("excluded_pattern=node_modules".to_string());
    lines.push("excluded_pattern=target".to_string());
    lines.push("excluded_pattern=dist".to_string());
    lines.push("excluded_pattern=build".to_string());
    lines.push("excluded_pattern=secrets".to_string());
    lines.push(String::new());
    let contents = lines.join("\n");
    let manifest_path = dir.join("manifest.txt");
    fs::write(&manifest_path, contents).expect("write manifest");
    manifest_path
}

fn make_recovery(source_path: &str, artifact_id: u64, title: &str) -> RecoveryInputs {
    RecoveryInputs {
        resume_parsers: vec![DomainInput::ResumeParser(ParserStarted {
            artifact_id: ArtifactId::new(artifact_id),
            title: title.to_string(),
            source_path: source_path.to_string(),
            content_hash: "sha256:aaa".to_string(),
            blob_id: BlobId::new(artifact_id),
        })],
        start_full_text: Vec::new(),
    }
}

#[test]
fn validate_recovery_scope_accepts_in_scope_source_paths() {
    let temp = TempDir::create();
    let root_str = temp.path().to_str().expect("temp path is valid UTF-8");
    write_manifest(temp.path(), &[root_str]);
    let layout = InstanceLayout::for_root(temp.path());

    let in_scope_path = temp.path().join("notes.md");
    fs::write(&in_scope_path, "# test").expect("write test file");

    let recovery = make_recovery(&in_scope_path.display().to_string(), 1, "notes.md");

    validate_recovery_scope(&layout, &recovery)
        .expect("in-scope source path should pass validation");
}

#[test]
fn validate_recovery_scope_rejects_out_of_scope_source_path() {
    let temp = TempDir::create();
    let root_str = temp.path().to_str().expect("temp path is valid UTF-8");
    write_manifest(temp.path(), &[root_str]);
    let layout = InstanceLayout::for_root(temp.path());

    let recovery = make_recovery("/tmp/outside.md", 1, "outside.md");

    let error = validate_recovery_scope(&layout, &recovery)
        .expect_err("out-of-scope source path should be rejected");
    let msg = format!("{error:#}");
    assert!(
        msg.contains("outside the instance manifest read scope"),
        "expected scope rejection for out-of-scope path, got: {msg}"
    );
    assert!(
        msg.contains("/tmp/outside.md"),
        "error should name the offending path, got: {msg}"
    );
    assert!(
        msg.contains("artifact 1 \"outside.md\""),
        "error should identify the artifact, got: {msg}"
    );
}

#[test]
fn validate_recovery_scope_rejects_excluded_source_path() {
    let temp = TempDir::create();
    let root_str = temp.path().to_str().expect("temp path is valid UTF-8");
    write_manifest(temp.path(), &[root_str]);
    let layout = InstanceLayout::for_root(temp.path());

    let excluded_path = temp.path().join(".env.local");
    let recovery = make_recovery(&excluded_path.display().to_string(), 2, ".env.local");

    let error = validate_recovery_scope(&layout, &recovery)
        .expect_err("excluded source path should be rejected");
    let msg = format!("{error:#}");
    assert!(
        msg.contains("outside the instance manifest read scope") || msg.contains("excluded"),
        "expected exclusion rejection for .env.local, got: {msg}"
    );
}

#[test]
fn validate_recovery_scope_empty_recovery_always_passes() {
    let temp = TempDir::create();
    write_manifest(temp.path(), &["/some/other/root"]);
    let layout = InstanceLayout::for_root(temp.path());

    let recovery = RecoveryInputs {
        resume_parsers: Vec::new(),
        start_full_text: Vec::new(),
    };

    validate_recovery_scope(&layout, &recovery)
        .expect("empty recovery should pass even with mismatched manifest");
}
