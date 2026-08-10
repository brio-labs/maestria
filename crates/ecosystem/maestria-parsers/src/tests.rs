use std::error::Error;
use std::path::PathBuf;

use maestria_domain::ArtifactId;
use maestria_ports::{FileHandle, FileMetadata, ParseContext, Parser, PortError};

use super::*;

fn handle(path: &str, bytes: &[u8]) -> FileHandle {
    FileHandle {
        path: PathBuf::from(path),
        bytes: bytes.to_vec(),
    }
}

fn context(id: u64) -> ParseContext {
    ParseContext {
        artifact_id: ArtifactId::new(id),
    }
}

fn metadata(path: &str, extension: Option<&str>) -> FileMetadata {
    FileMetadata {
        path: PathBuf::from(path),
        size: 0,
        extension: extension.map(str::to_string),
    }
}

#[test]
fn markdown_chunks_by_headings_and_creates_summary_card() -> Result<(), Box<dyn Error>> {
    let parsed = MarkdownParser::new().parse(
        handle("guide.md", b"intro\n\n# One\nalpha\n\n## Two\nbeta\n"),
        context(7),
    )?;

    assert_eq!(parsed.chunks.len(), 3);
    assert_eq!(parsed.chunks[0].text, "intro");
    assert_eq!(parsed.chunks[1].text, "# One\nalpha");
    assert_eq!(parsed.chunks[2].text, "## Two\nbeta");
    assert_eq!(
        parsed.chunks[0].chunk_id,
        chunk_id_for(ArtifactId::new(7), 0)?
    );
    assert_eq!(
        parsed.chunks[2].chunk_id,
        chunk_id_for(ArtifactId::new(7), 2)?
    );
    assert_eq!(parsed.cards.len(), 1);
    assert_eq!(
        parsed.cards[0].card.card_id,
        card_id_for(ArtifactId::new(7))
    );
    assert_eq!(parsed.cards[0].card.title, "intro");
    Ok(())
}

#[test]
fn plain_text_chunks_by_paragraph_groups() -> Result<(), Box<dyn Error>> {
    let parsed = PlainTextParser::new().parse(
        handle("notes.txt", b"alpha\ncontinues\n\n beta \n\n\n gamma"),
        context(3),
    )?;

    assert_eq!(parsed.chunks.len(), 3);
    assert_eq!(parsed.chunks[0].text, "alpha\ncontinues");
    assert_eq!(parsed.chunks[1].text, "beta");
    assert_eq!(parsed.chunks[2].text, "gamma");
    Ok(())
}

#[test]
fn rust_source_chunks_by_structural_starts_and_test_attributes() -> Result<(), Box<dyn Error>> {
    let parsed = RustSourceParser::new().parse(
            handle(
                "lib.rs",
                b"use std::fmt;\n\npub struct Thing;\n\nimpl Thing {\n    pub fn new() -> Self { Self }\n}\n\n#[test]\nfn makes_thing() {}\n",
            ),
            context(11),
        )?;

    assert_eq!(parsed.chunks.len(), 5);
    assert_eq!(parsed.chunks[0].text, "use std::fmt;");
    assert_eq!(parsed.chunks[1].text, "pub struct Thing;");
    assert_eq!(parsed.chunks[2].text, "impl Thing {");
    assert!(parsed.chunks[3].text.starts_with("pub fn new"));
    assert!(parsed.chunks[4].text.starts_with("#[test]\nfn makes_thing"));
    Ok(())
}
#[test]
fn cargo_toml_chunks_by_table_sections() -> Result<(), Box<dyn Error>> {
    let parsed = CargoTomlParser::new().parse(
            handle(
                "Cargo.toml",
                b"license = \"MIT\"\n\n[package]\nname = \"demo\"\n\n[dependencies]\nmaestria = \"0.1\"\n",
            ),
            context(5),
        )?;

    assert_eq!(parsed.chunks.len(), 3);
    assert_eq!(parsed.chunks[0].text, "license = \"MIT\"");
    assert_eq!(parsed.chunks[1].text, "[package]\nname = \"demo\"");
    assert_eq!(parsed.chunks[2].text, "[dependencies]\nmaestria = \"0.1\"");
    Ok(())
}
#[test]
fn registry_rejects_binary_content_by_sniffing() -> Result<(), Box<dyn Error>> {
    let registry = ParserRegistry::with_defaults();

    // Content decides, not the extension: binary bytes are rejected even
    // under a text-looking name, while ASCII text parses under a binary
    // extension.
    let binary = [0x89, 0x50, 0x4e, 0x47, 0x00, 0x0d, 0x0a, 0x1a];
    let res = registry.parse(handle("image.bin", &binary), context(13));
    assert!(matches!(res, Err(PortError::InvalidInputContext { .. })));
    let parsed = registry.parse(handle("image.bin", b"alpha"), context(14))?;
    assert_eq!(parsed.chunks[0].text, "alpha");
    Ok(())
}

#[test]
fn parser_rejects_invalid_utf8() {
    let res = PlainTextParser::new().parse(handle("notes.txt", &[0xff, 0xfe]), context(17));
    assert!(matches!(res, Err(PortError::InvalidInputContext { .. })));
}

#[test]
fn chunk_id_rejects_orders_outside_artifact_stride() {
    let res1 = chunk_id_for(ArtifactId::new(1), crate::chunking::ID_STRIDE as usize);
    assert!(matches!(res1, Err(PortError::InvalidInputContext { .. })));
    let res2 = chunk_id_for(ArtifactId::new(u64::MAX), 0);
    assert!(matches!(res2, Err(PortError::InvalidInputContext { .. })));
}
