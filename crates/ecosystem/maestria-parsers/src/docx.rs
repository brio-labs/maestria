#![forbid(unsafe_code)]

use std::io::{Cursor, Read};

use maestria_ports::{
    FileHandle, FileMetadata, ParseContext, ParsedArtifact, Parser, PortError, SourceSpan,
};
use quick_xml::{events::Event, name::ResolveResult, reader::NsReader};
use zip::ZipArchive;

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

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
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

#[derive(Debug)]
struct TextBlock {
    text: String,
    start_paragraph: usize,
    end_paragraph: usize,
}

#[derive(Default)]
struct TableRow {
    start_paragraph: Option<usize>,
    end_paragraph: Option<usize>,
    cells: Vec<String>,
}

#[derive(Default)]
struct Paragraph {
    number: usize,
    text: String,
}

#[derive(Clone, Copy)]
struct ArchiveFacts {
    entries: usize,
    document_xml_size: usize,
    content_types_size: usize,
}

fn extract_document_blocks(bytes: &[u8]) -> Result<Vec<TextBlock>, PortError> {
    let facts = inspect_archive(bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| invalid(format!("read DOCX ZIP directory: {error}")))?;
    if archive.len() != facts.entries {
        return Err(invalid("DOCX ZIP entry count changed during inspection"));
    }
    let content_types = read_part(
        &mut archive,
        "[Content_Types].xml",
        facts.content_types_size,
        MAX_DOCX_CONTENT_TYPES_BYTES,
    )?;
    validate_content_types(&content_types)?;
    let document = read_part(
        &mut archive,
        "word/document.xml",
        facts.document_xml_size,
        MAX_DOCX_DOCUMENT_XML_BYTES,
    )?;
    extract_paragraphs(&document)
}

fn inspect_archive(bytes: &[u8]) -> Result<ArchiveFacts, PortError> {
    if bytes.len() < 22 || bytes.len() > MAX_DOCX_ARCHIVE_BYTES {
        return Err(invalid(format!(
            "DOCX ZIP size must be between 22 and {MAX_DOCX_ARCHIVE_BYTES} bytes"
        )));
    }
    let search_start = bytes.len().saturating_sub(22 + usize::from(u16::MAX));
    let last_start = bytes.len() - 22;
    let eocd = (search_start..=last_start)
        .rev()
        .find(|offset| {
            read_u32(bytes, *offset) == Some(EOCD_SIGNATURE)
                && read_u16(bytes, offset + 20).is_some_and(|comment_len| {
                    offset
                        .checked_add(22 + usize::from(comment_len))
                        .is_some_and(|end| end == bytes.len())
                })
        })
        .ok_or_else(|| invalid("DOCX ZIP end directory is missing or malformed"))?;

    let disk_number =
        read_u16(bytes, eocd + 4).ok_or_else(|| invalid("truncated DOCX ZIP footer"))?;
    let directory_disk =
        read_u16(bytes, eocd + 6).ok_or_else(|| invalid("truncated DOCX ZIP footer"))?;
    let disk_entries =
        read_u16(bytes, eocd + 8).ok_or_else(|| invalid("truncated DOCX ZIP footer"))?;
    let entry_count =
        read_u16(bytes, eocd + 10).ok_or_else(|| invalid("truncated DOCX ZIP footer"))?;
    let directory_size =
        read_u32(bytes, eocd + 12).ok_or_else(|| invalid("truncated DOCX ZIP footer"))?;
    let directory_offset =
        read_u32(bytes, eocd + 16).ok_or_else(|| invalid("truncated DOCX ZIP footer"))?;

    if disk_number != 0 || directory_disk != 0 || disk_entries != entry_count {
        return Err(invalid("multi-disk DOCX ZIP archives are unsupported"));
    }
    if entry_count == u16::MAX || directory_size == u32::MAX || directory_offset == u32::MAX {
        return Err(invalid(
            "ZIP64 DOCX archives are unsupported by the bounded parser",
        ));
    }
    let entries = usize::from(entry_count);
    if entries == 0 || entries > MAX_DOCX_ENTRIES {
        return Err(invalid(format!(
            "DOCX ZIP entry count exceeds the limit of {MAX_DOCX_ENTRIES}"
        )));
    }
    let directory_size = usize::try_from(directory_size)
        .map_err(|error| invalid(format!("DOCX ZIP directory size is invalid: {error}")))?;
    let directory_offset = usize::try_from(directory_offset)
        .map_err(|error| invalid(format!("DOCX ZIP directory offset is invalid: {error}")))?;
    let directory_end = eocd;
    let directory_start = directory_end
        .checked_sub(directory_size)
        .ok_or_else(|| invalid("DOCX ZIP directory extends before the archive"))?;
    if directory_offset > directory_start {
        return Err(invalid("DOCX ZIP directory offset is inconsistent"));
    }

    let mut cursor = directory_start;
    let mut total_uncompressed = 0_u64;
    let mut document_xml = None;
    let mut content_types = None;
    for _ in 0..entries {
        if read_u32(bytes, cursor) != Some(CENTRAL_HEADER_SIGNATURE) {
            return Err(invalid("DOCX ZIP central directory entry is malformed"));
        }
        let flags = read_u16(bytes, cursor + 8).ok_or_else(|| invalid("truncated ZIP entry"))?;
        let method = read_u16(bytes, cursor + 10).ok_or_else(|| invalid("truncated ZIP entry"))?;
        let compressed_size =
            read_u32(bytes, cursor + 20).ok_or_else(|| invalid("truncated ZIP entry"))?;
        let uncompressed_size =
            read_u32(bytes, cursor + 24).ok_or_else(|| invalid("truncated ZIP entry"))?;
        let name_len = usize::from(
            read_u16(bytes, cursor + 28).ok_or_else(|| invalid("truncated ZIP entry"))?,
        );
        let extra_len = usize::from(
            read_u16(bytes, cursor + 30).ok_or_else(|| invalid("truncated ZIP entry"))?,
        );
        let comment_len = usize::from(
            read_u16(bytes, cursor + 32).ok_or_else(|| invalid("truncated ZIP entry"))?,
        );
        let disk_start =
            read_u16(bytes, cursor + 34).ok_or_else(|| invalid("truncated ZIP entry"))?;
        let local_header_offset =
            read_u32(bytes, cursor + 42).ok_or_else(|| invalid("truncated ZIP entry"))?;
        if compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || local_header_offset == u32::MAX
            || disk_start == u16::MAX
        {
            return Err(invalid(
                "ZIP64 DOCX entries are unsupported by the bounded parser",
            ));
        }
        if disk_start != 0 {
            return Err(invalid("multi-disk DOCX ZIP entries are unsupported"));
        }
        let fixed_end = cursor
            .checked_add(46)
            .ok_or_else(|| invalid("DOCX ZIP directory length overflow"))?;
        let name_start = fixed_end;
        let extra_start = name_start
            .checked_add(name_len)
            .ok_or_else(|| invalid("DOCX ZIP entry name length overflow"))?;
        let comment_start = extra_start
            .checked_add(extra_len)
            .ok_or_else(|| invalid("DOCX ZIP extra-field length overflow"))?;
        let record_end = comment_start
            .checked_add(comment_len)
            .ok_or_else(|| invalid("DOCX ZIP comment length overflow"))?;
        if record_end > directory_end || bytes.get(cursor..fixed_end).is_none() {
            return Err(invalid("DOCX ZIP central directory entry is truncated"));
        }
        let name = bytes
            .get(name_start..extra_start)
            .ok_or_else(|| invalid("DOCX ZIP entry name is truncated"))?;
        validate_extra_fields(bytes, extra_start, extra_len)?;
        let compressed_size = u64::from(compressed_size);
        let uncompressed_size = u64::from(uncompressed_size);
        total_uncompressed = total_uncompressed
            .checked_add(uncompressed_size)
            .ok_or_else(|| invalid("DOCX uncompressed size overflow"))?;
        if total_uncompressed > MAX_DOCX_UNCOMPRESSED_BYTES {
            return Err(invalid(format!(
                "DOCX total uncompressed size exceeds {MAX_DOCX_UNCOMPRESSED_BYTES} bytes"
            )));
        }

        if name == b"word/document.xml" || name == b"[Content_Types].xml" {
            if flags & ((1 << 0) | (1 << 6) | (1 << 13)) != 0 {
                return Err(invalid("encrypted DOCX XML parts are unsupported"));
            }
            if method != 0 && method != 8 {
                return Err(invalid(
                    "DOCX XML part uses an unsupported ZIP compression method",
                ));
            }
            if compressed_size > MAX_DOCX_ARCHIVE_BYTES as u64 {
                return Err(invalid(
                    "DOCX XML part compressed size exceeds the archive limit",
                ));
            }
            let destination = if name == b"word/document.xml" {
                &mut document_xml
            } else {
                &mut content_types
            };
            if destination.replace(uncompressed_size).is_some() {
                return Err(invalid("DOCX ZIP contains duplicate required XML parts"));
            }
        }
        cursor = record_end;
    }
    if cursor != directory_end {
        return Err(invalid(
            "DOCX ZIP central directory size does not match its entries",
        ));
    }
    let document_xml_size =
        usize::try_from(document_xml.ok_or_else(|| invalid("DOCX main document part is missing"))?)
            .map_err(|error| invalid(format!("DOCX document size is invalid: {error}")))?;
    if document_xml_size == 0 || document_xml_size > MAX_DOCX_DOCUMENT_XML_BYTES {
        return Err(invalid(format!(
            "DOCX document part exceeds the {} byte extraction limit",
            MAX_DOCX_DOCUMENT_XML_BYTES
        )));
    }
    let content_types_size = usize::try_from(
        content_types.ok_or_else(|| invalid("DOCX content-types part is missing"))?,
    )
    .map_err(|error| invalid(format!("DOCX content-types size is invalid: {error}")))?;
    if content_types_size == 0 || content_types_size > MAX_DOCX_CONTENT_TYPES_BYTES {
        return Err(invalid(format!(
            "DOCX content-types part exceeds the {} byte validation limit",
            MAX_DOCX_CONTENT_TYPES_BYTES
        )));
    }
    Ok(ArchiveFacts {
        entries,
        document_xml_size,
        content_types_size,
    })
}

fn validate_extra_fields(bytes: &[u8], start: usize, length: usize) -> Result<(), PortError> {
    let end = start
        .checked_add(length)
        .ok_or_else(|| invalid("DOCX ZIP extra-field length overflow"))?;
    let mut cursor = start;
    while cursor < end {
        if end - cursor < 4 {
            return Err(invalid("DOCX ZIP extra field is truncated"));
        }
        let field_id =
            read_u16(bytes, cursor).ok_or_else(|| invalid("truncated ZIP extra field"))?;
        let field_size = usize::from(
            read_u16(bytes, cursor + 2).ok_or_else(|| invalid("truncated ZIP extra field"))?,
        );
        if field_id == 0x0001 {
            return Err(invalid("ZIP64 DOCX extra fields are unsupported"));
        }
        cursor = cursor
            .checked_add(4 + field_size)
            .filter(|next| *next <= end)
            .ok_or_else(|| invalid("DOCX ZIP extra field extends past its entry"))?;
    }
    Ok(())
}

fn read_part(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &str,
    expected_size: usize,
    maximum_size: usize,
) -> Result<Vec<u8>, PortError> {
    let part = archive
        .by_name(name)
        .map_err(|error| invalid(format!("read DOCX part {name}: {error}")))?;
    if part.size() != expected_size as u64 || expected_size > maximum_size {
        return Err(invalid(format!("DOCX part {name} exceeds its size limit")));
    }
    let mut output = Vec::with_capacity(expected_size);
    part.take(maximum_size as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|error| invalid(format!("decompress DOCX part {name}: {error}")))?;
    if output.len() != expected_size {
        return Err(invalid(format!(
            "DOCX part {name} decompressed size does not match its ZIP metadata"
        )));
    }
    Ok(output)
}

fn validate_content_types(bytes: &[u8]) -> Result<(), PortError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| invalid(format!("DOCX content-types XML is not UTF-8: {error}")))?;
    if !bytes
        .windows(CONTENT_TYPES_MAIN_DOCUMENT.len())
        .any(|window| window == CONTENT_TYPES_MAIN_DOCUMENT)
        || !bytes
            .windows(DOCUMENT_PART_NAME.len())
            .any(|window| window == DOCUMENT_PART_NAME)
    {
        return Err(invalid(
            "DOCX content-types does not declare its Word document part",
        ));
    }
    let mut reader = quick_xml::reader::Reader::from_reader(bytes);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut events = 0;
    let mut root_seen = false;
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| invalid(format!("DOCX content-types XML parse error: {error}")))?;
        events += 1;
        if events > MAX_DOCX_XML_EVENTS {
            return Err(invalid("DOCX content-types XML event limit exceeded"));
        }
        match event {
            Event::DocType(_) => {
                return Err(invalid("DOCX content-types XML may not contain a DTD"));
            }
            Event::Start(start) => {
                if !root_seen {
                    root_seen = true;
                    if start.local_name().as_ref() != b"Types" {
                        return Err(invalid("DOCX content-types XML root is not Types"));
                    }
                }
            }
            Event::Empty(empty) if !root_seen => {
                root_seen = true;
                if empty.local_name().as_ref() != b"Types" {
                    return Err(invalid("DOCX content-types XML root is not Types"));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root_seen {
        return Err(invalid("DOCX content-types XML has no root element"));
    }
    let _ = text;
    Ok(())
}

fn extract_paragraphs(xml: &[u8]) -> Result<Vec<TextBlock>, PortError> {
    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut blocks = Vec::new();
    let mut table_rows = Vec::<TableRow>::new();
    let mut cells = Vec::<String>::new();
    let mut paragraph = None;
    let mut in_text = false;
    let mut in_body = false;
    let mut table_depth = 0_usize;
    let mut paragraph_count = 0_usize;
    let mut decoded_text_bytes = 0_usize;
    let mut output_text_bytes = 0_usize;
    let mut xml_depth = 0_usize;
    let mut event_count = 0_usize;
    let mut root_seen = false;
    let mut body_seen = false;

    loop {
        let (namespace, event) = reader
            .read_resolved_event_into(&mut buffer)
            .map_err(|error| invalid(format!("DOCX document XML parse error: {error}")))?;
        event_count += 1;
        if event_count > MAX_DOCX_XML_EVENTS {
            return Err(invalid("DOCX document XML event limit exceeded"));
        }
        match event {
            Event::Start(start) => {
                xml_depth += 1;
                if xml_depth > MAX_DOCX_XML_DEPTH {
                    return Err(invalid("DOCX document XML nesting limit exceeded"));
                }
                let local = start.local_name();
                let word_element = is_word_namespace(&namespace);
                if !root_seen {
                    root_seen = true;
                    if !word_element || local.as_ref() != b"document" {
                        return Err(invalid("DOCX document XML root is not a Word document"));
                    }
                }
                if word_element {
                    match local.as_ref() {
                        b"body" => {
                            if in_body {
                                return Err(invalid("DOCX document contains a nested body"));
                            }
                            in_body = true;
                            body_seen = true;
                        }
                        b"tbl" if in_body => table_depth += 1,
                        b"tr" if in_body && table_depth > 0 => table_rows.push(TableRow::default()),
                        b"tc" if in_body && table_depth > 0 && !table_rows.is_empty() => {
                            cells.push(String::new());
                        }
                        b"p" if in_body => {
                            begin_paragraph(&mut paragraph, &mut paragraph_count, &mut table_rows)?;
                        }
                        b"t" if paragraph.is_some() => in_text = true,
                        b"tab" if paragraph.is_some() => {
                            append_paragraph_text(&mut paragraph, "\t", &mut decoded_text_bytes)?;
                        }
                        b"br" | b"cr" if paragraph.is_some() => {
                            append_paragraph_text(&mut paragraph, "\n", &mut decoded_text_bytes)?;
                        }
                        _ => {}
                    }
                }
            }
            Event::Empty(empty) => {
                let local = empty.local_name();
                if is_word_namespace(&namespace) {
                    match local.as_ref() {
                        b"p" if in_body => {
                            begin_paragraph(&mut paragraph, &mut paragraph_count, &mut table_rows)?;
                            finish_paragraph(
                                &mut paragraph,
                                &mut table_rows,
                                &mut cells,
                                &mut blocks,
                                &mut output_text_bytes,
                            )?;
                        }
                        b"tab" if paragraph.is_some() => {
                            append_paragraph_text(&mut paragraph, "\t", &mut decoded_text_bytes)?;
                        }
                        b"br" | b"cr" if paragraph.is_some() => {
                            append_paragraph_text(&mut paragraph, "\n", &mut decoded_text_bytes)?;
                        }
                        _ => {}
                    }
                }
            }
            Event::End(end) => {
                let local = end.local_name();
                if is_word_namespace(&namespace) {
                    match local.as_ref() {
                        b"t" => in_text = false,
                        b"p" if in_body => finish_paragraph(
                            &mut paragraph,
                            &mut table_rows,
                            &mut cells,
                            &mut blocks,
                            &mut output_text_bytes,
                        )?,
                        b"tc" if in_body && table_depth > 0 => {
                            let cell = cells
                                .pop()
                                .ok_or_else(|| invalid("DOCX table cell is not balanced"))?;
                            table_rows
                                .last_mut()
                                .ok_or_else(|| invalid("DOCX table row is not balanced"))?
                                .cells
                                .push(cell);
                        }
                        b"tr" if in_body && table_depth > 0 => {
                            finish_table_row(
                                &mut table_rows,
                                &mut cells,
                                &mut blocks,
                                &mut output_text_bytes,
                            )?;
                        }
                        b"tbl" if in_body => {
                            table_depth = table_depth
                                .checked_sub(1)
                                .ok_or_else(|| invalid("DOCX table depth is not balanced"))?;
                        }
                        b"body" => {
                            if paragraph.is_some() || !table_rows.is_empty() || !cells.is_empty() {
                                return Err(invalid("DOCX body ended inside a paragraph or table"));
                            }
                            in_body = false;
                        }
                        _ => {}
                    }
                }
                xml_depth = xml_depth
                    .checked_sub(1)
                    .ok_or_else(|| invalid("DOCX XML end tag has no matching start tag"))?;
            }
            Event::Text(text) if in_text => {
                let decoded = text
                    .xml10_content()
                    .map_err(|error| invalid(format!("decode DOCX paragraph text: {error}")))?;
                append_paragraph_text(&mut paragraph, &decoded, &mut decoded_text_bytes)?;
            }
            Event::CData(text) if in_text => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid(format!("decode DOCX paragraph CDATA: {error}")))?;
                append_paragraph_text(&mut paragraph, &decoded, &mut decoded_text_bytes)?;
            }
            Event::GeneralRef(reference) if in_text => {
                let name = reference
                    .decode()
                    .map_err(|error| invalid(format!("decode DOCX XML entity: {error}")))?;
                let character = if reference.is_char_ref() {
                    reference
                        .resolve_char_ref()
                        .map_err(|error| {
                            invalid(format!("resolve DOCX character reference: {error}"))
                        })?
                        .ok_or_else(|| invalid("invalid DOCX character reference"))?
                } else {
                    match name.as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "apos" => '\'',
                        "quot" => '"',
                        _ => return Err(invalid("DOCX XML contains an undeclared entity")),
                    }
                };
                let mut encoded = [0_u8; 4];
                append_paragraph_text(
                    &mut paragraph,
                    character.encode_utf8(&mut encoded),
                    &mut decoded_text_bytes,
                )?;
            }
            Event::DocType(_) => return Err(invalid("DOCX document XML may not contain a DTD")),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root_seen || !body_seen || in_body || xml_depth != 0 {
        return Err(invalid("DOCX document XML is incomplete"));
    }
    if paragraph.is_some() || !table_rows.is_empty() || !cells.is_empty() || table_depth != 0 {
        return Err(invalid(
            "DOCX document XML ended inside a paragraph or table",
        ));
    }
    Ok(blocks)
}

fn begin_paragraph(
    paragraph: &mut Option<Paragraph>,
    paragraph_count: &mut usize,
    table_rows: &mut [TableRow],
) -> Result<(), PortError> {
    if paragraph.is_some() {
        return Err(invalid("DOCX document contains nested paragraphs"));
    }
    *paragraph_count = paragraph_count
        .checked_add(1)
        .ok_or_else(|| invalid("DOCX paragraph count overflow"))?;
    if *paragraph_count > MAX_DOCX_PARAGRAPHS {
        return Err(invalid(format!(
            "DOCX paragraph count exceeds the limit of {MAX_DOCX_PARAGRAPHS}"
        )));
    }
    for row in table_rows {
        row.start_paragraph.get_or_insert(*paragraph_count);
        row.end_paragraph = Some(*paragraph_count);
    }
    *paragraph = Some(Paragraph {
        number: *paragraph_count,
        text: String::new(),
    });
    Ok(())
}

fn append_paragraph_text(
    paragraph: &mut Option<Paragraph>,
    fragment: &str,
    total_bytes: &mut usize,
) -> Result<(), PortError> {
    let Some(paragraph) = paragraph.as_mut() else {
        return Ok(());
    };
    *total_bytes = total_bytes
        .checked_add(fragment.len())
        .ok_or_else(|| invalid("DOCX extracted text size overflow"))?;
    if *total_bytes > MAX_DOCX_TEXT_BYTES {
        return Err(invalid(format!(
            "DOCX extracted text exceeds the {MAX_DOCX_TEXT_BYTES} byte limit"
        )));
    }
    paragraph.text.push_str(fragment);
    Ok(())
}

fn finish_paragraph(
    paragraph: &mut Option<Paragraph>,
    table_rows: &mut [TableRow],
    cells: &mut [String],
    blocks: &mut Vec<TextBlock>,
    output_text_bytes: &mut usize,
) -> Result<(), PortError> {
    let paragraph = paragraph
        .take()
        .ok_or_else(|| invalid("DOCX paragraph end has no matching start"))?;
    let text = paragraph.text.trim();
    if text.is_empty() {
        return Ok(());
    }
    if let Some(cell) = cells.last_mut() {
        append_cell_text(cell, text)?;
    } else if let Some(row) = table_rows.last_mut() {
        row.cells.push(text.to_owned());
    } else {
        push_block(
            blocks,
            text.to_owned(),
            paragraph.number,
            paragraph.number,
            output_text_bytes,
        )?;
    }
    Ok(())
}

fn finish_table_row(
    table_rows: &mut Vec<TableRow>,
    cells: &mut [String],
    blocks: &mut Vec<TextBlock>,
    output_text_bytes: &mut usize,
) -> Result<(), PortError> {
    let row = table_rows
        .pop()
        .ok_or_else(|| invalid("DOCX table row end has no matching start"))?;
    let Some(start_paragraph) = row.start_paragraph else {
        return Ok(());
    };
    let Some(end_paragraph) = row.end_paragraph else {
        return Err(invalid("DOCX table row has no paragraph end"));
    };
    let text = row
        .cells
        .iter()
        .map(|cell| cell.trim())
        .collect::<Vec<_>>()
        .join(" | ");
    if text.trim().is_empty() {
        return Ok(());
    }
    if let Some(parent_cell) = cells.last_mut() {
        append_cell_text(parent_cell, &text)?;
    } else {
        push_block(
            blocks,
            text,
            start_paragraph,
            end_paragraph,
            output_text_bytes,
        )?;
    }
    Ok(())
}

fn append_cell_text(cell: &mut String, text: &str) -> Result<(), PortError> {
    let additional = text.len() + usize::from(!cell.is_empty());
    if cell.len().saturating_add(additional) > MAX_DOCX_TEXT_BYTES {
        return Err(invalid(format!(
            "DOCX table text exceeds the {MAX_DOCX_TEXT_BYTES} byte limit"
        )));
    }
    if !cell.is_empty() {
        cell.push('\n');
    }
    cell.push_str(text);
    Ok(())
}

fn push_block(
    blocks: &mut Vec<TextBlock>,
    text: String,
    start_paragraph: usize,
    end_paragraph: usize,
    output_text_bytes: &mut usize,
) -> Result<(), PortError> {
    if blocks.len() >= MAX_DOCX_PARAGRAPHS {
        return Err(invalid(format!(
            "DOCX extracted block count exceeds the limit of {MAX_DOCX_PARAGRAPHS}"
        )));
    }
    *output_text_bytes = output_text_bytes
        .checked_add(text.len())
        .and_then(|bytes| bytes.checked_add(1))
        .ok_or_else(|| invalid("DOCX output text size overflow"))?;
    if *output_text_bytes > MAX_DOCX_TEXT_BYTES {
        return Err(invalid(format!(
            "DOCX output text exceeds the {MAX_DOCX_TEXT_BYTES} byte limit"
        )));
    }
    blocks.push(TextBlock {
        text,
        start_paragraph,
        end_paragraph,
    });
    Ok(())
}

fn is_word_namespace(namespace: &ResolveResult<'_>) -> bool {
    match namespace {
        ResolveResult::Bound(namespace) => {
            namespace.as_ref() == WORD_NAMESPACE || namespace.as_ref() == STRICT_WORD_NAMESPACE
        }
        ResolveResult::Unbound | ResolveResult::Unknown(_) => false,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    Some(u16::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
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

    use super::{
        CONTENT_TYPES_MAIN_DOCUMENT, DOCUMENT_PART_NAME, DocxParser, MAX_DOCX_DOCUMENT_XML_BYTES,
        MAX_DOCX_ENTRIES, WORD_NAMESPACE,
    };

    fn document(contents: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="{}"><w:body>{contents}</w:body></w:document>"#,
            std::str::from_utf8(WORD_NAMESPACE).unwrap_or("invalid")
        )
    }

    fn make_docx(document_xml: &str, extra_entries: usize) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        archive.start_file("[Content_Types].xml", options)?;
        archive.write_all(
            format!(
                r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="{}" ContentType="application/vnd.openxmlformats-officedocument.{}"/></Types>"#,
                std::str::from_utf8(DOCUMENT_PART_NAME).unwrap_or("invalid"),
                std::str::from_utf8(CONTENT_TYPES_MAIN_DOCUMENT).unwrap_or("invalid"),
            )
            .as_bytes(),
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
