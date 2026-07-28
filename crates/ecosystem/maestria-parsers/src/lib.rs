#![forbid(unsafe_code)]

//! Deterministic byte-to-domain parsers for Maestria artifacts.

/// Responsibility map:
/// - `cargo_toml`: module responsibility.
/// - `chunking`: module responsibility.
/// - `markdown`: module responsibility.
/// - `pdf`: module responsibility.
/// - `pdf_geometry`: PDF page geometry, transform, and bounds utilities.
/// - `pdf_layout`: PDF region extraction and layout building.
/// - `plain_text`: module responsibility.
/// - `registry`: module responsibility.
/// - `rust_source`: module responsibility.
/// - `tree_builder`: module responsibility.
mod cargo_toml;
mod chunking;
mod markdown;
mod pdf;
mod pdf_geometry;
mod pdf_layout;
mod plain_text;
mod registry;
mod rust_source;
mod tree_builder;

pub use cargo_toml::CargoTomlParser;
pub use chunking::{card_id_for, chunk_id_for};
pub use markdown::MarkdownParser;
pub use pdf::PdfParser;
pub use plain_text::PlainTextParser;
pub use registry::ParserRegistry;
pub use rust_source::RustSourceParser;

#[cfg(test)]
mod tests;
