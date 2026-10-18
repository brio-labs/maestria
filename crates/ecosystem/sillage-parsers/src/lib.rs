#![forbid(unsafe_code)]

//! Deterministic byte-to-domain parsers for Sillage artifacts.

/// Responsibility map:
/// - `artifact_builder`: ordinary parsed-artifact and summary-card assembly.
/// - `bounded_markdown`: cooperative bounded UTF-8 Markdown source scanning.
/// - `bounded_output`: budget-admitted artifact, summary-card, and hash assembly.
/// - `bounded_path`: budgeted native path filename and extension scanning.
/// - `bounded_summary`: budget-admitted summary-card text formatting.
/// - `cargo_toml`: module responsibility.
/// - `chunking`: module responsibility.
/// - `docx`: bounded DOCX ZIP/XML text extraction.
/// - `generic_text`: module responsibility.
/// - `markdown`: module responsibility.
/// - `pdf`: module responsibility.
/// - `pdf_geometry`: PDF page geometry, transform, and bounds utilities.
/// - `pdf_layout`: PDF region extraction and layout building.
/// - `pdf_tree`: module responsibility.
/// - `plain_text`: module responsibility.
/// - `python_source`: module responsibility.
/// - `registry`: module responsibility.
/// - `rust_source`: module responsibility.
/// - `tree_builder`: module responsibility.
/// - `typescript_source`: module responsibility.
mod artifact_builder;
mod bounded_markdown;
mod bounded_output;
mod bounded_path;
mod bounded_summary;
mod cargo_toml;
mod chunking;
mod docx;
mod generic_text;
mod markdown;
mod pdf;
mod pdf_geometry;
mod pdf_layout;
mod pdf_tree;
mod plain_text;
mod python_source;
mod registry;
mod rust_source;
mod tree_builder;
mod typescript_source;

pub use cargo_toml::CargoTomlParser;
pub use chunking::{card_id_for, chunk_id_for};
pub use docx::DocxParser;
pub use generic_text::GenericTextParser;
pub use markdown::MarkdownParser;
pub use pdf::PdfParser;
pub use plain_text::PlainTextParser;
pub use python_source::PythonSourceParser;
pub use registry::ParserRegistry;
pub use rust_source::RustSourceParser;
pub use typescript_source::TypeScriptSourceParser;

#[cfg(test)]
mod tests;
