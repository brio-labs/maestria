use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use maestria_blob_fs::FsBlobStore;
use maestria_core::InstanceLayout;
use maestria_domain::{ArtifactId, BlobId, DomainInput, KernelState, ParserStarted, content_hash};
use maestria_ports::BlobStore;

use super::parser_resume::{pending_resume_parsers, verify_pending_blobs};

/// A temporary directory that is removed on drop. Each instance gets a unique
/// path via an atomic counter so tests running in the same process do not collide.
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("maestria-parser-resume-test-{id}"));
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

fn pending_input(
    state: &mut KernelState,
    artifact_id: u64,
    blob_id: u64,
    title: &str,
    content_hash: &str,
) -> DomainInput {
    let artifact_id = ArtifactId::new(artifact_id);
    let blob_id = BlobId::new(blob_id);
    state.pending_parsers.insert(
        artifact_id,
        ParserStarted {
            artifact_id,
            title: title.to_string(),
            source_path: format!("/tmp/{title}"),
            content_hash: content_hash.to_string(),
            blob_id,
        },
    );
    DomainInput::ResumeParser(ParserStarted {
        artifact_id,
        title: title.to_string(),
        source_path: format!("/tmp/{title}"),
        content_hash: content_hash.to_string(),
        blob_id,
    })
}

fn build_layout(tempdir: &TempDir) -> InstanceLayout {
    InstanceLayout::for_root(tempdir.path())
}

/// Creates a layout and blob store, puts bytes, and returns the blob id.
fn put_blob(layout: &InstanceLayout, bytes: &[u8]) -> BlobId {
    let store = FsBlobStore::open(&layout.blobs_dir).expect("open blob store for test fixture");
    store
        .put(bytes.to_vec())
        .expect("put blob for test fixture")
}

#[test]
fn pending_resume_parsers_empty_when_nothing_pending() {
    let state = KernelState::new();
    let inputs = pending_resume_parsers(&state);
    assert!(inputs.is_empty());
}

#[test]
fn pending_resume_parsers_collects_all_pending() {
    let mut state = KernelState::new();
    let artifact_a = ArtifactId::new(1);
    let artifact_b = ArtifactId::new(2);
    let blob_a = BlobId::new(100);
    let blob_b = BlobId::new(200);

    state.pending_parsers.insert(
        artifact_a,
        ParserStarted {
            artifact_id: artifact_a,
            title: "a.md".to_string(),
            source_path: "/tmp/a.md".to_string(),
            content_hash: "sha256:aaa".to_string(),
            blob_id: blob_a,
        },
    );
    state.pending_parsers.insert(
        artifact_b,
        ParserStarted {
            artifact_id: artifact_b,
            title: "b.md".to_string(),
            source_path: "/tmp/b.md".to_string(),
            content_hash: "sha256:bbb".to_string(),
            blob_id: blob_b,
        },
    );

    let inputs = pending_resume_parsers(&state);
    assert_eq!(inputs.len(), 2);

    let mut ids: Vec<ArtifactId> = inputs
        .iter()
        .map(|input| match input {
            DomainInput::ResumeParser(ps) => ps.artifact_id,
            other => panic!("expected ResumeParser, got {other:?}"),
        })
        .collect();
    ids.sort();
    assert_eq!(ids, vec![artifact_a, artifact_b]);
}

#[test]
fn pending_resume_parsers_preserves_all_fields() {
    let mut state = KernelState::new();
    let artifact_id = ArtifactId::new(42);
    let blob_id = BlobId::new(999);

    state.pending_parsers.insert(
        artifact_id,
        ParserStarted {
            artifact_id,
            title: "report.pdf".to_string(),
            source_path: "/data/report.pdf".to_string(),
            content_hash: "sha256:abcdef1234567890".to_string(),
            blob_id,
        },
    );

    let inputs = pending_resume_parsers(&state);
    assert_eq!(inputs.len(), 1);

    match &inputs[0] {
        DomainInput::ResumeParser(ps) => {
            assert_eq!(ps.artifact_id, artifact_id);
            assert_eq!(ps.title, "report.pdf");
            assert_eq!(ps.source_path, "/data/report.pdf");
            assert_eq!(ps.content_hash, "sha256:abcdef1234567890");
            assert_eq!(ps.blob_id, blob_id);
        }
        other => panic!("expected ResumeParser, got {other:?}"),
    }
}

// --- verify_pending_blobs tests ---

#[test]
fn verify_pending_blobs_ok_when_empty() {
    let tempdir = TempDir::new();
    let layout = build_layout(&tempdir);
    assert!(verify_pending_blobs(&layout, &[]).is_ok());
}

#[test]
fn verify_pending_blobs_succeeds_when_all_blobs_exist() {
    let tempdir = TempDir::new();
    let layout = build_layout(&tempdir);

    let blob_id_a = put_blob(&layout, b"alpha");
    let blob_id_b = put_blob(&layout, b"beta");

    let input_a = DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(blob_id_a.value()),
        title: "a.txt".to_string(),
        source_path: "/tmp/a.txt".to_string(),
        content_hash: content_hash(b"alpha"),
        blob_id: blob_id_a,
    });
    let input_b = DomainInput::ResumeParser(ParserStarted {
        artifact_id: ArtifactId::new(blob_id_b.value()),
        title: "b.txt".to_string(),
        source_path: "/tmp/b.txt".to_string(),
        content_hash: content_hash(b"beta"),
        blob_id: blob_id_b,
    });

    verify_pending_blobs(&layout, &[input_a, input_b]).expect("all blobs should be found");
}

#[test]
fn verify_pending_blobs_fails_when_blob_missing() {
    let tempdir = TempDir::new();
    let layout = build_layout(&tempdir);

    // Never put blob 42 — it should fail.
    let mut state = KernelState::new();
    let input = pending_input(&mut state, 1, 42, "missing.txt", "sha256:0");

    let err = verify_pending_blobs(&layout, &[input]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("missing or corrupt"),
        "error should mention missing or corrupt blob, got: {msg}"
    );
}

#[test]
fn verify_pending_blobs_fails_when_object_file_missing_but_index_present() {
    let tempdir = TempDir::new();
    let layout = build_layout(&tempdir);

    let blob_id = put_blob(&layout, b"doomed");

    // Remove the object file but keep the index mapping.
    {
        let store = FsBlobStore::open(&layout.blobs_dir).expect("open");
        let digest = store.digest_for_id(blob_id).expect("digest lookup");
        let object_path = store.object_path_for_digest(&digest).expect("object path");
        fs::remove_file(&object_path).expect("remove object file");
    }

    let mut state = KernelState::new();
    let hash = content_hash(b"doomed");
    let input = pending_input(
        &mut state,
        blob_id.value(),
        blob_id.value(),
        "doomed.txt",
        &hash,
    );

    let err = verify_pending_blobs(&layout, &[input]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("missing or corrupt"),
        "error should mention missing or corrupt blob when object file is gone, got: {msg}"
    );
}

#[test]
fn verify_pending_blobs_fails_when_content_hash_mismatch() {
    let tempdir = TempDir::new();
    let layout = build_layout(&tempdir);

    let bytes = b"alpha";
    let blob_id = put_blob(&layout, bytes);

    // Construct a ParserStarted that references the correct blob_id
    // but carries a content hash for *different* bytes.
    let actual_hash = content_hash(bytes);
    let wrong_hash = content_hash(b"beta"); // hash of different content
    assert_ne!(actual_hash, wrong_hash, "test requires distinct hashes");

    let artifact_id = ArtifactId::new(1);
    let input = DomainInput::ResumeParser(ParserStarted {
        artifact_id,
        title: "tampered.txt".to_string(),
        source_path: "/tmp/tampered.txt".to_string(),
        content_hash: wrong_hash.clone(),
        blob_id,
    });

    let mut state = KernelState::new();
    state.pending_parsers.insert(
        artifact_id,
        ParserStarted {
            artifact_id,
            title: "tampered.txt".to_string(),
            source_path: "/tmp/tampered.txt".to_string(),
            content_hash: wrong_hash.clone(),
            blob_id,
        },
    );

    let err = verify_pending_blobs(&layout, &[input]).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("content hash mismatch"),
        "error should mention content hash mismatch, got: {msg}"
    );
}
