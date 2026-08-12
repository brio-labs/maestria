//! Content-based generic text parser.
//!
//! Extension-independent fallback: any source under the size cap that
//! actually looks like UTF-8 text is indexed as plain paragraphs, so
//! JSON, YAML, shell, CSS, configs, and extension-less files become
//! searchable when they hold real text — and binaries and huge
//! minified bundles are rejected by content, not by an extension table.

#![forbid(unsafe_code)]

use maestria_ports::{FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError};

use crate::chunking::{decode_utf8, paragraph_chunks, parsed_artifact};

/// Largest source accepted as generic text. Larger files are left to
/// specialized parsers or terminalized as unsupported.
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// Control bytes that legitimately appear in text sources.
fn is_text_byte(byte: u8) -> bool {
    byte >= 0x20 || matches!(byte, b'\n' | b'\r' | b'\t' | 0x0b | 0x0c)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GenericTextParser;

impl GenericTextParser {
    pub const fn new() -> Self {
        Self
    }

    /// Binary sniffing over the leading sample: any NUL byte or a
    /// disproportionate share of non-text control bytes means the
    /// source is not prose the user would search.
    fn looks_like_text(bytes: &[u8]) -> bool {
        if bytes.contains(&0) {
            return false;
        }
        let sample = &bytes[..bytes.len().min(8192)];
        let suspicious = sample.iter().filter(|byte| !is_text_byte(**byte)).count();
        suspicious * 100 <= sample.len() * 5
    }
}

impl Parser for GenericTextParser {
    fn id(&self) -> &'static str {
        "generic-text-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.size <= MAX_TEXT_BYTES
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        if !Self::looks_like_text(&file.bytes) {
            return Err(PortError::InvalidInputContext {
                context: "generic text parse",
                source: "file content is not text".to_string(),
            });
        }
        let bytes = file.bytes.clone();
        let text = decode_utf8(file.bytes)?;
        let chunks = paragraph_chunks(&text);
        parsed_artifact(
            context.artifact_id,
            &file.path,
            &bytes,
            chunks,
            self.id().to_string(),
            "1.0".to_string(),
            Some("text".to_string()),
        )
    }
}
