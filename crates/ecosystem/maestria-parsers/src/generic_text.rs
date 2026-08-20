//! Content-based generic text parser.

#![forbid(unsafe_code)]

use crate::chunking::{MAX_TEXT_BYTES, looks_like_text, paragraph_chunks};
use crate::text_parser;

fn generic_precheck(bytes: &[u8]) -> Result<(), maestria_ports::PortError> {
    if !looks_like_text(bytes) {
        return Err(maestria_ports::PortError::InvalidInputContext {
            context: "generic text parse",
            source: "file content is not text".to_string(),
        });
    }
    Ok(())
}

text_parser!(
    GenericTextParser,
    "generic-text-parser",
    |file: &maestria_ports::FileMetadata| file.size <= MAX_TEXT_BYTES,
    "generic-text-parser",
    "1.0",
    "text",
    paragraph_chunks,
    generic_precheck
);
