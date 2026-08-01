use super::super::contract_tests::*;
use super::super::*;
use maestria_domain::ArtifactId;
use std::path::PathBuf;

#[test]
fn in_memory_parser_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_parser_round_trip(
        &InMemoryParser::new(),
        &FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"alpha".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[test]
fn in_memory_parser_multiline_source_span() -> Result<(), Box<dyn std::error::Error>> {
    let parser = InMemoryParser::new();
    let parsed = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"line one\nline two\nline three".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(1),
        },
    )?;
    assert_eq!(parsed.chunks.len(), 1);
    match parsed.chunks[0].source_span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => {
            assert_eq!(start_line, 1);
            assert_eq!(end_line, 3, "expected end_line == 3 for three-line input");
        }
        _ => return Err("expected TextSpan".into()),
    }
    Ok(())
}

#[test]
fn in_memory_parser_version_id_changes_with_content() -> Result<(), Box<dyn std::error::Error>> {
    let parser = InMemoryParser::new();
    let artifact_id = ArtifactId::new(42);
    let first = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"first draft".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    let second = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"second draft".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    assert_ne!(
        first.artifact_version_id, second.artifact_version_id,
        "same artifact with different bytes must yield different version ids"
    );
    Ok(())
}

#[test]
fn in_memory_parser_version_id_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let parser = InMemoryParser::new();
    let artifact_id = ArtifactId::new(42);
    let first = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"stable content".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    let second = parser.parse(
        FileHandle {
            path: PathBuf::from("notes.md"),
            bytes: b"stable content".to_vec(),
        },
        ParseContext { artifact_id },
    )?;
    assert_eq!(
        first.artifact_version_id, second.artifact_version_id,
        "same artifact with identical bytes must yield identical version ids"
    );
    Ok(())
}
