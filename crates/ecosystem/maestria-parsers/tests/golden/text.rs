use std::error::Error;
use std::path::PathBuf;

use maestria_domain::ArtifactId;
use maestria_parsers::*;
use maestria_ports::{FileHandle, ParseContext, Parser};

#[test]
fn markdown_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input =
        b"# Title\n\nIntro paragraph.\n\n## Section 1\nContent here.\n\n### Subsection\nMore content.\n";
    let parsed = MarkdownParser::new().parse(
        FileHandle {
            path: PathBuf::from("guide.md"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(1),
        },
    )?;
    crate::assert_debug_snapshot(
        "markdown_parsed",
        &parsed,
        concat!(module_path!(), "::markdown_golden_snapshot"),
        file!(),
        stringify!(&parsed),
        line!(),
    )?;
    Ok(())
}

#[test]
fn plain_text_golden_snapshot() -> Result<(), Box<dyn Error>> {
    let input = b"First paragraph.\nStill first.\n\nSecond paragraph.\n";
    let parsed = PlainTextParser::new().parse(
        FileHandle {
            path: PathBuf::from("notes.txt"),
            bytes: input.to_vec(),
        },
        ParseContext {
            artifact_id: ArtifactId::new(2),
        },
    )?;
    crate::assert_debug_snapshot(
        "plain_text_parsed",
        &parsed,
        concat!(module_path!(), "::plain_text_golden_snapshot"),
        file!(),
        stringify!(&parsed),
        line!(),
    )?;
    Ok(())
}
