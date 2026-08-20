#![forbid(unsafe_code)]

use crate::chunking::{extension_is, paragraph_chunks};
use crate::text_parser;

text_parser!(
    PlainTextParser,
    "plain-text-parser",
    |file: &maestria_ports::FileMetadata| extension_is(file, &["txt", "text"]),
    "plain-text-parser",
    "1.0",
    "text",
    paragraph_chunks
);
