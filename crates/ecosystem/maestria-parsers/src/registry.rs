use maestria_ports::{
    FileHandle, FileMetadata, ParseContext, ParseOutcome, ParsedArtifact, Parser, PortError,
};

use crate::cargo_toml::CargoTomlParser;
use crate::chunking::metadata_for_handle;
use crate::generic_text::GenericTextParser;
use crate::markdown::MarkdownParser;
use crate::pdf::PdfParser;
use crate::plain_text::PlainTextParser;
use crate::python_source::PythonSourceParser;
use crate::rust_source::RustSourceParser;
use crate::typescript_source::TypeScriptSourceParser;

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
        registry.register(PythonSourceParser::new());
        registry.register(TypeScriptSourceParser::new());
        registry.register(CargoTomlParser::new());
        registry.register(PdfParser::new());
        // Extension-independent fallback: claims any unclaimed source
        // within the text size cap; content decides in `parse`.
        registry.register(GenericTextParser::new());
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

    fn resolve_parser(&self, file: &FileHandle) -> Result<&dyn Parser, PortError> {
        let metadata = metadata_for_handle(file);
        self.parser_for(&metadata).ok_or_else(|| {
            PortError::invalid_input(
                "unsupported file extension",
                file.path.display().to_string(),
            )
        })
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
        let parser = self.resolve_parser(&file)?;
        parser.parse(file, context)
    }

    fn parse_outcome(
        &self,
        file: FileHandle,
        context: ParseContext,
    ) -> Result<ParseOutcome, PortError> {
        let parser = self.resolve_parser(&file)?;
        parser.parse_outcome(file, context)
    }

    fn parse_with_ocr(
        &self,
        file: FileHandle,
        context: ParseContext,
        pages: &[maestria_domain::OcrPageText],
    ) -> Result<ParsedArtifact, PortError> {
        let parser = self.resolve_parser(&file)?;
        parser.parse_with_ocr(file, context, pages)
    }
}
