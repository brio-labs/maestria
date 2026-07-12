use std::path::PathBuf;

use maestria_core::{CoreError, build_artifact_detected_input};
use maestria_domain::{ArtifactDetected, DomainInput};

#[test]
fn empty_bytes_triggers_invalid_input_error() {
    let result = build_artifact_detected_input(&PathBuf::from("notes/empty.md"), vec![]);
    assert!(
        matches!(result, Err(CoreError::InvalidInput { .. })),
        "empty bytes must produce an InvalidInput error, got {result:?}"
    );
}

#[test]
fn deterministic_artifact_id_and_content_hash_from_same_input() {
    let path = PathBuf::from("notes/project.md");
    let bytes = b"# Project Notes\n\nFirst evidence block.\n".to_vec();

    let first = build_artifact_detected_input(&path, bytes.clone())
        .expect("valid input must produce DomainInput");
    let second = build_artifact_detected_input(&path, bytes.clone())
        .expect("same input must produce DomainInput");
    let third =
        build_artifact_detected_input(&path, bytes).expect("same input must produce DomainInput");

    // All three must be identical — pure determinism.
    assert_eq!(first, second);
    assert_eq!(second, third);

    // Verify the constructed DomainInput variant and fields.
    let DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id,
        title,
        source_path,
        source_bytes,
        content_hash,
    }) = &first
    else {
        panic!("expected ArtifactDetected, got {first:?}");
    };
    assert_eq!(source_path, "notes/project.md");
    assert_eq!(title, "project.md");
    assert!(!source_bytes.is_empty());
    assert!(!content_hash.is_empty());
    // artifact_id is deterministic for the given path+bytes fixture.
    assert_eq!(artifact_id.value(), 421114891);
}

#[test]
fn different_bytes_produce_different_ids() {
    let path = PathBuf::from("notes/a.md");
    let a = build_artifact_detected_input(&path, b"alpha".to_vec()).expect("valid input");
    let b = build_artifact_detected_input(&path, b"beta".to_vec()).expect("valid input");

    // Different content → different DomainInput.
    assert_ne!(a, b);
}

#[test]
fn different_paths_produce_different_ids() {
    let bytes = b"same content".to_vec();
    let a = build_artifact_detected_input(&PathBuf::from("notes/one.md"), bytes.clone())
        .expect("valid input");
    let b = build_artifact_detected_input(&PathBuf::from("notes/two.md"), bytes.clone())
        .expect("valid input");

    assert_ne!(a, b);
}

#[test]
fn title_falls_back_to_artifact_when_path_has_no_filename() {
    let result = build_artifact_detected_input(&PathBuf::from("."), b"content".to_vec())
        .expect("valid input");
    let DomainInput::ArtifactDetected(ArtifactDetected { title, .. }) = &result else {
        panic!("expected ArtifactDetected");
    };
    // title_for_path returns "artifact" when filename is absent or empty.
    assert_eq!(title, "artifact");
}
