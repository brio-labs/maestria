#![forbid(unsafe_code)]

//! Deterministic byte-to-domain parsers for Maestria artifacts.

mod cargo_toml;
mod chunking;
mod markdown;
mod pdf;
mod plain_text;
mod registry;
mod rust_source;

pub use cargo_toml::CargoTomlParser;
pub use chunking::{card_id_for, chunk_id_for};
pub use markdown::MarkdownParser;
pub use pdf::PdfParser;
pub use plain_text::PlainTextParser;
pub use registry::ParserRegistry;
pub use rust_source::RustSourceParser;

#[cfg(test)]
mod tests {
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
    fn markdown_chunks_by_headings_and_creates_summary_card() {
        let parsed = MarkdownParser::new()
            .parse(
                handle("guide.md", b"intro\n\n# One\nalpha\n\n## Two\nbeta\n"),
                context(7),
            )
            .expect("markdown parses");

        assert_eq!(parsed.chunks.len(), 3);
        assert_eq!(parsed.chunks[0].text, "intro");
        assert_eq!(parsed.chunks[1].text, "# One\nalpha");
        assert_eq!(parsed.chunks[2].text, "## Two\nbeta");
        assert_eq!(
            parsed.chunks[0].chunk_id,
            chunk_id_for(ArtifactId::new(7), 0).expect("chunk id")
        );
        assert_eq!(
            parsed.chunks[2].chunk_id,
            chunk_id_for(ArtifactId::new(7), 2).expect("chunk id")
        );
        assert_eq!(parsed.cards.len(), 1);
        assert_eq!(parsed.cards[0].card_id, card_id_for(ArtifactId::new(7)));
        assert_eq!(parsed.cards[0].title, "intro");
    }

    #[test]
    fn plain_text_chunks_by_paragraph_groups() {
        let parsed = PlainTextParser::new()
            .parse(
                handle("notes.txt", b"alpha\ncontinues\n\n beta \n\n\n gamma"),
                context(3),
            )
            .expect("plain text parses");

        assert_eq!(parsed.chunks.len(), 3);
        assert_eq!(parsed.chunks[0].text, "alpha\ncontinues");
        assert_eq!(parsed.chunks[1].text, "beta");
        assert_eq!(parsed.chunks[2].text, "gamma");
    }

    #[test]
    fn rust_source_chunks_by_structural_starts_and_test_attributes() {
        let parsed = RustSourceParser::new()
            .parse(
                handle(
                    "lib.rs",
                    b"use std::fmt;\n\npub struct Thing;\n\nimpl Thing {\n    pub fn new() -> Self { Self }\n}\n\n#[test]\nfn makes_thing() {}\n",
                ),
                context(11),
            )
            .expect("rust source parses");

        assert_eq!(parsed.chunks.len(), 5);
        assert_eq!(parsed.chunks[0].text, "use std::fmt;");
        assert_eq!(parsed.chunks[1].text, "pub struct Thing;");
        assert_eq!(parsed.chunks[2].text, "impl Thing {");
        assert!(parsed.chunks[3].text.starts_with("pub fn new"));
        assert!(parsed.chunks[4].text.starts_with("#[test]\nfn makes_thing"));
    }

    #[test]
    fn cargo_toml_chunks_by_table_sections() {
        let parsed = CargoTomlParser::new()
            .parse(
                handle(
                    "Cargo.toml",
                    b"license = \"MIT\"\n\n[package]\nname = \"demo\"\n\n[dependencies]\nmaestria = \"0.1\"\n",
                ),
                context(5),
            )
            .expect("cargo toml parses");

        assert_eq!(parsed.chunks.len(), 3);
        assert_eq!(parsed.chunks[0].text, "license = \"MIT\"");
        assert_eq!(parsed.chunks[1].text, "[package]\nname = \"demo\"");
        assert_eq!(parsed.chunks[2].text, "[dependencies]\nmaestria = \"0.1\"");
    }

    #[test]
    fn registry_rejects_unsupported_extension() {
        let registry = ParserRegistry::with_defaults();

        assert!(!registry.supports(&metadata("image.bin", Some("bin"))));
        assert!(matches!(
            registry.parse(handle("image.bin", b"alpha"), context(13)),
            Err(PortError::InvalidInput { .. })
        ));
    }

    #[test]
    fn parser_rejects_invalid_utf8() {
        assert!(matches!(
            PlainTextParser::new().parse(handle("notes.txt", &[0xff, 0xfe]), context(17)),
            Err(PortError::InvalidInput { .. })
        ));
    }

    #[test]
    fn chunk_id_rejects_orders_outside_artifact_stride() {
        assert!(matches!(
            chunk_id_for(ArtifactId::new(1), crate::chunking::ID_STRIDE as usize),
            Err(PortError::InvalidInput { .. })
        ));
        assert!(matches!(
            chunk_id_for(ArtifactId::new(u64::MAX), 0),
            Err(PortError::InvalidInput { .. })
        ));
    }
}
