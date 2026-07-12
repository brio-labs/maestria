use maestria_domain::{ArtifactId, BlobId, DomainInput, KernelState, ParserStarted};

use super::parser_resume::pending_resume_parsers;

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
