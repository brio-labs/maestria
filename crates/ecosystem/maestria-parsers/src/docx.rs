#![forbid(unsafe_code)]

mod archive;
mod content_types;
mod mapping;
mod paragraphs;
mod part_reader;

use archive::extract_document_blocks;
use maestria_ports::{
    FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError, SourceSpan,
};

use crate::chunking::parsed_artifact;

const MAX_DOCX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOCX_ENTRIES: usize = 512;
const MAX_DOCX_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DOCX_DOCUMENT_XML_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCX_CONTENT_TYPES_BYTES: usize = 256 * 1024;
const MAX_DOCX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_DOCX_XML_EVENTS: usize = 100_000;
const MAX_DOCX_XML_DEPTH: usize = 256;
const MAX_DOCX_PARAGRAPHS: usize = 100_000;

const WORD_NAMESPACE: &[u8] = b"http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const STRICT_WORD_NAMESPACE: &[u8] = b"http://purl.oclc.org/ooxml/wordprocessingml/main";
const CONTENT_TYPES_MAIN_DOCUMENT: &[u8] = b"wordprocessingml.document.main+xml";
const DOCUMENT_PART_NAME: &[u8] = b"/word/document.xml";

#[derive(Clone, Default)]
pub struct DocxParser;

impl DocxParser {
    pub fn new() -> Self {
        Self
    }

    fn parse_docx(
        &self,
        file: FileHandle,
        context: ParseContext,
    ) -> Result<ParsedArtifact, PortError> {
        let blocks = extract_document_blocks(&file.bytes)?;
        if blocks.is_empty() {
            return Err(invalid("DOCX document contains no extractable text"));
        }
        let chunks = blocks
            .into_iter()
            .map(|block| {
                (
                    block.text,
                    SourceSpan::DocxParagraphSpan {
                        start_paragraph: block.start_paragraph,
                        end_paragraph: block.end_paragraph,
                    },
                )
            })
            .collect();
        parsed_artifact(
            context.artifact_id,
            &file.path,
            &file.bytes,
            chunks,
            "docx-parser-1".to_owned(),
            "1".to_owned(),
            None,
        )
    }
}

impl Parser for DocxParser {
    fn id(&self) -> &'static str {
        "docx-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.extension
            .as_deref()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"))
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        self.parse_docx(file, context)
    }
}

fn invalid(source: impl Into<String>) -> PortError {
    PortError::InvalidInputContext {
        context: "DOCX parse error",
        source: source.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::io::{Cursor, Write};

    use maestria_domain::ArtifactId;
    use maestria_ports::{FileHandle, ParseContext, Parser, SourceSpan};
    use zip::write::FileOptions;

    use super::{DocxParser, MAX_DOCX_DOCUMENT_XML_BYTES, MAX_DOCX_ENTRIES};

    fn document(contents: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{contents}</w:body></w:document>"#
        )
    }

    fn make_docx(document_xml: &str, extra_entries: usize) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        archive.start_file("[Content_Types].xml", options)?;
        archive.write_all(
            br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        )?;
        archive.start_file("word/document.xml", options)?;
        archive.write_all(document_xml.as_bytes())?;
        for index in 0..extra_entries {
            archive.start_file(format!("extra-{index}.bin"), options)?;
        }
        Ok(archive.finish()?.into_inner())
    }

    fn handle(bytes: Vec<u8>) -> FileHandle {
        FileHandle {
            path: "approved/report.docx".into(),
            bytes,
        }
    }

    fn context(id: u64) -> ParseContext {
        ParseContext {
            artifact_id: ArtifactId::new(id),
        }
    }

    #[test]
    fn extracts_paragraphs_and_table_rows_with_truthful_ordinals() -> Result<(), Box<dyn Error>> {
        let xml = document(
            "<w:p><w:r><w:t>Budget &amp; spend</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Revenue</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2026</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
        );
        let parsed = DocxParser::new().parse(handle(make_docx(&xml, 0)?), context(12))?;
        assert_eq!(parsed.chunks.len(), 2);
        assert_eq!(parsed.chunks[0].text, "Budget & spend");
        assert_eq!(parsed.chunks[1].text, "Revenue | 2026");
        assert_eq!(
            parsed.chunks[0].source_span,
            SourceSpan::DocxParagraphSpan {
                start_paragraph: 1,
                end_paragraph: 1,
            }
        );
        assert_eq!(
            parsed.chunks[1].source_span,
            SourceSpan::DocxParagraphSpan {
                start_paragraph: 2,
                end_paragraph: 3,
            }
        );
        Ok(())
    }

    #[test]
    fn rejects_zip_with_too_many_entries_before_extraction() -> Result<(), Box<dyn Error>> {
        let xml = document("<w:p><w:r><w:t>bounded</w:t></w:r></w:p>");
        let bytes = make_docx(&xml, MAX_DOCX_ENTRIES)?;
        let parsed = DocxParser::new().parse(handle(bytes), context(13));
        match parsed {
            Ok(_) => Err("over-entry-limit DOCX was parsed".into()),
            Err(error) => {
                assert!(error.to_string().contains("entry count exceeds"));
                Ok(())
            }
        }
    }

    #[test]
    fn rejects_oversized_document_xml_before_decompression() -> Result<(), Box<dyn Error>> {
        let contents = "x".repeat(MAX_DOCX_DOCUMENT_XML_BYTES);
        let xml = document(&format!("<w:p><w:r><w:t>{contents}</w:t></w:r></w:p>"));
        let bytes = make_docx(&xml, 0)?;
        let parsed = DocxParser::new().parse(handle(bytes), context(14));
        match parsed {
            Ok(_) => Err("over-limit DOCX document XML was parsed".into()),
            Err(error) => {
                assert!(error.to_string().contains("document part exceeds"));
                Ok(())
            }
        }
    }
}
