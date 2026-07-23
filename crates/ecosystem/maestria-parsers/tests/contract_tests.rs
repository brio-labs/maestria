mod common;

use std::error::Error;
use std::path::PathBuf;

use common::*;
use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::contract_tests::assert_parser_round_trip;
use maestria_ports::{FileHandle, ParseContext};

// ── shared contract suite (Rule 25) ────────────────────────────────

#[test]
fn markdown_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &MarkdownParser::new(),
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
fn plain_text_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &PlainTextParser::new(),
        &FileHandle {
            path: PathBuf::from("notes.txt"),
            bytes: b"alpha".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[test]
fn rust_source_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &RustSourceParser::new(),
        &FileHandle {
            path: PathBuf::from("lib.rs"),
            bytes: b"fn main() {}".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[test]
fn cargo_toml_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &CargoTomlParser::new(),
        &FileHandle {
            path: PathBuf::from("Cargo.toml"),
            bytes: b"[package]\nname = \"test\"".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[test]
fn pdf_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &PdfParser::new(),
        &FileHandle {
            path: PathBuf::from("document.pdf"),
            bytes: create_minimal_pdf(b"alpha")?,
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}
