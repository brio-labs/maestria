#![forbid(unsafe_code)]

//! Deterministic byte-to-domain parsers for Maestria artifacts.
mod pdf;
pub use pdf::PdfParser;

use std::path::Path;

use maestria_domain::{ArtifactId, CardId, ChunkId, CreateCardInput};
use maestria_ports::{
    FileHandle, FileMetadata, ParseContext, ParsedArtifact, ParsedChunk, Parser, PortError,
    SourceSpan,
};

const ID_STRIDE: u64 = 1_000_003;
const CARD_OFFSET: u64 = 900_001;

#[derive(Default)]
pub struct ParserRegistry {
    parsers: Vec<Box<dyn Parser + Send + Sync>>,
}

impl ParserRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        registry.register(MarkdownParser::new());
        registry.register(PlainTextParser::new());
        registry.register(RustSourceParser::new());
        registry.register(CargoTomlParser::new());
        registry.register(PdfParser::new());
        registry
    }

    pub fn register<P>(&mut self, parser: P)
    where
        P: Parser + Send + Sync + 'static,
    {
        self.parsers.push(Box::new(parser));
    }

    pub fn parser_for(&self, file: &FileMetadata) -> Option<&dyn Parser> {
        self.parsers
            .iter()
            .map(Box::as_ref)
            .find(|parser| parser.supports(file))
            .map(|parser| parser as &dyn Parser)
    }

    pub fn parser_count(&self) -> usize {
        self.parsers.len()
    }
}

impl Parser for ParserRegistry {
    fn id(&self) -> &'static str {
        "parser-registry"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        self.parser_for(file).is_some()
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let metadata = metadata_for_handle(&file);
        let parser = self
            .parser_for(&metadata)
            .ok_or_else(|| PortError::InvalidInput {
                message: format!("unsupported file extension for {}", file.path.display()),
            })?;
        parser.parse(file, context)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownParser;

impl MarkdownParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for MarkdownParser {
    fn id(&self) -> &'static str {
        "markdown-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["md", "markdown"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = markdown_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PlainTextParser;

impl PlainTextParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for PlainTextParser {
    fn id(&self) -> &'static str {
        "plain-text-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["txt", "text"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = paragraph_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RustSourceParser;

impl RustSourceParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for RustSourceParser {
    fn id(&self) -> &'static str {
        "rust-source-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        extension_is(file, &["rs"])
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = rust_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CargoTomlParser;

impl CargoTomlParser {
    pub const fn new() -> Self {
        Self
    }
}

impl Parser for CargoTomlParser {
    fn id(&self) -> &'static str {
        "cargo-toml-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("Cargo.toml"))
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = decode_utf8(file.bytes)?;
        let chunks = cargo_toml_chunks(&text);
        parsed_artifact(context.artifact_id, &file.path, chunks)
    }
}

pub fn chunk_id_for(artifact_id: ArtifactId, chunk_order: usize) -> Result<ChunkId, PortError> {
    if chunk_order as u64 >= ID_STRIDE {
        return Err(PortError::InvalidInput {
            message: format!("chunk order {chunk_order} exceeds parser id stride {ID_STRIDE}"),
        });
    }

    let id = artifact_id
        .value()
        .checked_mul(ID_STRIDE)
        .and_then(|value| value.checked_add(chunk_order as u64))
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| PortError::InvalidInput {
            message: format!(
                "artifact id {} cannot be expanded into deterministic chunk ids",
                artifact_id.value()
            ),
        })?;

    Ok(ChunkId::new(id))
}

pub fn card_id_for(artifact_id: ArtifactId) -> CardId {
    CardId::new(
        artifact_id
            .value()
            .wrapping_mul(ID_STRIDE)
            .wrapping_add(CARD_OFFSET),
    )
}

fn metadata_for_handle(file: &FileHandle) -> FileMetadata {
    FileMetadata {
        path: file.path.clone(),
        size: file.bytes.len(),
        extension: file
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase),
    }
}

fn extension_is(file: &FileMetadata, accepted: &[&str]) -> bool {
    file.extension.as_deref().is_some_and(|extension| {
        accepted
            .iter()
            .any(|accepted| extension.eq_ignore_ascii_case(accepted))
    })
}

fn decode_utf8(bytes: Vec<u8>) -> Result<String, PortError> {
    if bytes.is_empty() {
        return Err(PortError::InvalidInput {
            message: "input file is empty".to_string(),
        });
    }

    String::from_utf8(bytes).map_err(|err| PortError::InvalidInput {
        message: format!("file bytes are not utf8: {err}"),
    })
}

fn parsed_artifact(
    artifact_id: ArtifactId,
    path: &Path,
    chunks_with_spans: Vec<(String, SourceSpan)>,
) -> Result<ParsedArtifact, PortError> {
    if chunks_with_spans.is_empty() {
        return Err(PortError::InvalidInput {
            message: "input file has no textual content".to_string(),
        });
    }

    let mut chunks = Vec::with_capacity(chunks_with_spans.len());
    for (order, (text, source_span)) in chunks_with_spans.into_iter().enumerate() {
        chunks.push(ParsedChunk {
            chunk_id: chunk_id_for(artifact_id, order)?,
            artifact_id,
            text,
            source_span,
        });
    }

    let card = summary_card_for(artifact_id, path, &chunks);
    Ok(ParsedArtifact {
        artifact_id,
        chunks,
        cards: vec![card],
    })
}

pub(crate) fn summary_card_for(
    artifact_id: ArtifactId,
    path: &Path,
    chunks: &[ParsedChunk],
) -> CreateCardInput {
    let first_line = chunks
        .first()
        .and_then(|chunk| chunk.text.lines().find(|line| !line.trim().is_empty()))
        .map(clean_summary_line)
        .filter(|line| !line.is_empty());
    let fallback_title = match path.file_name().and_then(|name| name.to_str()) {
        Some(name) => name.to_string(),
        None => "artifact".to_string(),
    };
    let title = match first_line {
        Some(line) => line,
        None => fallback_title,
    };
    let unit = if chunks.len() == 1 { "chunk" } else { "chunks" };

    CreateCardInput {
        card_id: card_id_for(artifact_id),
        artifact_id,
        title,
        body: format!(
            "Parsed {} textual {} from {}.",
            chunks.len(),
            unit,
            path.display()
        ),
    }
}

fn clean_summary_line(line: &str) -> String {
    let trimmed = line.trim().trim_start_matches('#').trim();
    trimmed.chars().take(96).collect()
}

fn markdown_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let heading_lines = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| is_markdown_heading(line).then_some(index))
        .collect::<Vec<_>>();

    if heading_lines.is_empty() {
        return paragraph_chunks(text);
    }

    ranges_from_starts(text, heading_lines)
}

fn is_markdown_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let marker_len = trimmed.chars().take_while(|ch| *ch == '#').count();
    (1..=6).contains(&marker_len)
        && match trimmed.chars().nth(marker_len) {
            Some(ch) => ch.is_whitespace(),
            None => true,
        }
}

fn paragraph_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let mut chunks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut para_start: Option<usize> = None;

    for (line_idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            if let Some(start) = para_start.take() {
                let joined = current.join("\n").trim().to_string();
                current.clear();
                if !joined.is_empty() {
                    chunks.push((
                        joined,
                        SourceSpan::TextSpan {
                            start_line: start + 1,
                            end_line: line_idx,
                        },
                    ));
                }
            }
        } else {
            if para_start.is_none() {
                para_start = Some(line_idx);
            }
            current.push(line);
        }
    }
    if let Some(start) = para_start.take() {
        let joined = current.join("\n").trim().to_string();
        if !joined.is_empty() {
            let total_lines = text.lines().count();
            chunks.push((
                joined,
                SourceSpan::TextSpan {
                    start_line: start + 1,
                    end_line: total_lines,
                },
            ));
        }
    }

    chunks
}

fn rust_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let mut starts = Vec::new();
    let mut pending_attribute_start = None;

    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("#[") {
            pending_attribute_start.get_or_insert(index);
            continue;
        }

        if is_rust_structural_start(trimmed) {
            let start = match pending_attribute_start.take() {
                Some(attribute_start) => attribute_start,
                None => index,
            };
            starts.push(start);
        } else if !trimmed.is_empty() && !trimmed.starts_with("//") {
            pending_attribute_start = None;
        }
    }

    if starts.is_empty() {
        return paragraph_chunks(text);
    }

    starts.sort_unstable();
    starts.dedup();
    ranges_from_starts(text, starts)
}

fn is_rust_structural_start(trimmed: &str) -> bool {
    let without_visibility = match trimmed.strip_prefix("pub ") {
        Some(stripped) => stripped,
        None => trimmed,
    };
    let without_async = match without_visibility.strip_prefix("async ") {
        Some(stripped) => stripped,
        None => without_visibility,
    };
    without_async.starts_with("fn ")
        || without_async.starts_with("struct ")
        || without_async.starts_with("enum ")
        || without_async.starts_with("trait ")
        || without_visibility.starts_with("impl")
}

fn cargo_toml_chunks(text: &str) -> Vec<(String, SourceSpan)> {
    let starts = text
        .lines()
        .enumerate()
        .filter_map(|(index, line)| is_toml_table_header(line).then_some(index))
        .collect::<Vec<_>>();

    if starts.is_empty() {
        return paragraph_chunks(text);
    }

    ranges_from_starts(text, starts)
}

fn is_toml_table_header(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('[') && trimmed.ends_with(']')
}

fn ranges_from_starts(text: &str, starts: Vec<usize>) -> Vec<(String, SourceSpan)> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut chunks = Vec::new();

    if let Some(first_start) = starts.first().copied() {
        push_range(&mut chunks, &lines, 0, first_start);
    }

    for (position, start) in starts.iter().copied().enumerate() {
        let end = match starts.get(position + 1).copied() {
            Some(next_start) => next_start,
            None => lines.len(),
        };
        push_range(&mut chunks, &lines, start, end);
    }

    chunks
}

fn push_range(chunks: &mut Vec<(String, SourceSpan)>, lines: &[&str], start: usize, end: usize) {
    if start >= end {
        return;
    }

    let text = lines[start..end].join("\n").trim().to_string();
    if !text.is_empty() {
        chunks.push((
            text,
            SourceSpan::TextSpan {
                start_line: start + 1,
                end_line: end,
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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
            chunk_id_for(ArtifactId::new(1), ID_STRIDE as usize),
            Err(PortError::InvalidInput { .. })
        ));
        assert!(matches!(
            chunk_id_for(ArtifactId::new(u64::MAX), 0),
            Err(PortError::InvalidInput { .. })
        ));
    }
}
