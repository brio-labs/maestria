mod common;

use std::error::Error;
use std::path::PathBuf;

use common::*;
use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::contract_tests::assert_parser_round_trip;
use maestria_ports::{FileHandle, ParseContext, Parser};

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
fn python_source_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &PythonSourceParser::new(),
        &FileHandle {
            path: PathBuf::from("app.py"),
            bytes: b"def main():\n    return 42".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[test]
fn typescript_source_parser_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &TypeScriptSourceParser::new(),
        &FileHandle {
            path: PathBuf::from("app.ts"),
            bytes: b"export function main() {\n  return 42;\n}".to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    Ok(())
}

#[test]
fn typescript_source_chunks_tile_the_file_contiguously() -> Result<(), Box<dyn Error>> {
    // A TSX file with imports, a decorator above a class, an interface, a
    // function, and a const: every line belongs to exactly one chunk and the
    // chunks cover the file with no gaps and no overlaps.
    let source = "\
import { Button } from \"./button\";

@Component
export class List {
  render() {
    return <ul />;
  }
}

export interface ListProps {
  items: string[];
}

export function makeList(items: string[]) {
  return items;
}

const MAX_ITEMS = 10;
";
    let parser = TypeScriptSourceParser::new();
    assert_eq!(parser.id(), "typescript-source-parser");
    let artifact = parser.parse(
        FileHandle {
            path: PathBuf::from("list.tsx"),
            bytes: source.as_bytes().to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    // One header chunk (lines before the first declaration) plus one chunk
    // per declaration (decorator included).
    assert_eq!(
        artifact.chunks.len(),
        5,
        "expected header plus declaration chunks"
    );
    let total_lines = source.lines().count();
    let mut next_start = 1;
    let mut previous_end = 0;
    for chunk in &artifact.chunks {
        let maestria_ports::SourceSpan::TextSpan {
            start_line,
            end_line,
        } = chunk.source_span
        else {
            return Err("expected text span".into());
        };
        assert!(
            start_line > previous_end,
            "chunk overlaps or gaps before line {start_line}"
        );
        assert_eq!(
            start_line, next_start,
            "chunk must start where the last ended"
        );
        next_start = end_line + 1;
        previous_end = end_line;
    }
    assert_eq!(
        next_start,
        total_lines + 1,
        "chunks must cover every line of the file"
    );
    Ok(())
}

#[test]
fn python_source_chunks_tile_the_file_contiguously() -> Result<(), Box<dyn Error>> {
    // A python file with a decorator, a class, methods, a module-level
    // function, and an async function: every line belongs to exactly one
    // chunk and the chunks cover the file with no gaps and no overlaps.
    let source = "\
import os

@staticmethod
class Greeter:
    def __init__(self, name):
        self.name = name

    def hello(self):
        return f\"hello {self.name}\"

def make_greeter(name):
    return Greeter(name)

async def fetch(url):
    return os.getenv(url)
";
    let parser = PythonSourceParser::new();
    assert_eq!(parser.id(), "python-source-parser");
    let artifact = parser.parse(
        FileHandle {
            path: PathBuf::from("greeter.py"),
            bytes: source.as_bytes().to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(7),
        },
    )?;
    // One header chunk (lines before the first declaration) plus one chunk
    // per declaration (decorator included).
    assert_eq!(
        artifact.chunks.len(),
        6,
        "expected header plus declaration chunks"
    );
    let total_lines = source.lines().count();
    let mut next_start = 1;
    let mut previous_end = 0;
    for chunk in &artifact.chunks {
        let maestria_ports::SourceSpan::TextSpan {
            start_line,
            end_line,
        } = chunk.source_span
        else {
            return Err("expected text span".into());
        };
        assert!(
            start_line > previous_end,
            "chunk overlaps or gaps before line {start_line}"
        );
        assert_eq!(
            start_line, next_start,
            "chunk must start where the last ended"
        );
        next_start = end_line + 1;
        previous_end = end_line;
    }
    assert_eq!(
        next_start,
        total_lines + 1,
        "chunks must cover every line of the file"
    );
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

#[test]
fn parser_registry_satisfies_contract() -> Result<(), Box<dyn Error>> {
    assert_parser_round_trip(
        &ParserRegistry::with_defaults(),
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
