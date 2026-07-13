#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::cargo_toml::CargoTomlParser;
use crate::chunking::metadata_for_handle;
use crate::markdown::MarkdownParser;
use crate::pdf::PdfParser;
use crate::plain_text::PlainTextParser;
use crate::rust_source::RustSourceParser;

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
