use quick_xml::{
    events::{BytesRef, Event},
    name::ResolveResult,
    reader::NsReader,
};
use sillage_ports::PortError;

use super::mapping::{BlockMapper, TextBlock};
use super::{
    MAX_DOCX_XML_DEPTH, MAX_DOCX_XML_EVENTS, STRICT_WORD_NAMESPACE, WORD_NAMESPACE, invalid,
};

#[derive(Default)]
struct ParagraphExtractor {
    mapping: BlockMapper,
    in_text: bool,
    in_body: bool,
    table_depth: usize,
    xml_depth: usize,
    event_count: usize,
    root_seen: bool,
    body_seen: bool,
}

pub(super) fn extract_paragraphs(xml: &[u8]) -> Result<Vec<TextBlock>, PortError> {
    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut extractor = ParagraphExtractor::default();
    loop {
        let (namespace, event) = reader
            .read_resolved_event_into(&mut buffer)
            .map_err(|error| invalid(format!("DOCX document XML parse error: {error}")))?;
        extractor.record_event()?;
        match event {
            Event::Start(start) => {
                extractor.start_element(&namespace, start.local_name().as_ref())?;
            }
            Event::Empty(empty) => {
                extractor.empty_element(&namespace, empty.local_name().as_ref())?;
            }
            Event::End(end) => {
                extractor.end_element(&namespace, end.local_name().as_ref())?;
            }
            Event::Text(text) if extractor.in_text => {
                let decoded = text
                    .xml10_content()
                    .map_err(|error| invalid(format!("decode DOCX paragraph text: {error}")))?;
                extractor.mapping.append_text(&decoded)?;
            }
            Event::CData(text) if extractor.in_text => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid(format!("decode DOCX paragraph CDATA: {error}")))?;
                extractor.mapping.append_text(&decoded)?;
            }
            Event::GeneralRef(reference) if extractor.in_text => {
                let character = decode_reference(&reference)?;
                let mut encoded = [0_u8; 4];
                extractor
                    .mapping
                    .append_text(character.encode_utf8(&mut encoded))?;
            }
            Event::DocType(_) => return Err(invalid("DOCX document XML may not contain a DTD")),
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    extractor.finish()
}

impl ParagraphExtractor {
    fn record_event(&mut self) -> Result<(), PortError> {
        self.event_count += 1;
        if self.event_count > MAX_DOCX_XML_EVENTS {
            return Err(invalid("DOCX document XML event limit exceeded"));
        }
        Ok(())
    }

    fn start_element(
        &mut self,
        namespace: &ResolveResult<'_>,
        local: &[u8],
    ) -> Result<(), PortError> {
        self.xml_depth += 1;
        if self.xml_depth > MAX_DOCX_XML_DEPTH {
            return Err(invalid("DOCX document XML nesting limit exceeded"));
        }
        let word_element = is_word_namespace(namespace);
        if !self.root_seen {
            self.root_seen = true;
            if !word_element || local != b"document" {
                return Err(invalid("DOCX document XML root is not a Word document"));
            }
        }
        if word_element {
            match local {
                b"body" => {
                    if self.in_body {
                        return Err(invalid("DOCX document contains a nested body"));
                    }
                    self.in_body = true;
                    self.body_seen = true;
                }
                b"tbl" if self.in_body => self.table_depth += 1,
                b"tr" if self.in_body && self.table_depth > 0 => self.mapping.start_row(),
                b"tc" if self.in_body && self.table_depth > 0 && self.mapping.has_open_row() => {
                    self.mapping.start_cell();
                }
                b"p" if self.in_body => self.mapping.begin_paragraph()?,
                b"t" if self.mapping.has_open_paragraph() => self.in_text = true,
                b"tab" if self.mapping.has_open_paragraph() => {
                    self.mapping.append_text("\t")?;
                }
                b"br" | b"cr" if self.mapping.has_open_paragraph() => {
                    self.mapping.append_text("\n")?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn empty_element(
        &mut self,
        namespace: &ResolveResult<'_>,
        local: &[u8],
    ) -> Result<(), PortError> {
        if !is_word_namespace(namespace) {
            return Ok(());
        }
        match local {
            b"p" if self.in_body => {
                self.mapping.begin_paragraph()?;
                self.mapping.finish_paragraph()?;
            }
            b"tab" if self.mapping.has_open_paragraph() => self.mapping.append_text("\t")?,
            b"br" | b"cr" if self.mapping.has_open_paragraph() => {
                self.mapping.append_text("\n")?;
            }
            _ => {}
        }
        Ok(())
    }

    fn end_element(
        &mut self,
        namespace: &ResolveResult<'_>,
        local: &[u8],
    ) -> Result<(), PortError> {
        if is_word_namespace(namespace) {
            match local {
                b"t" => self.in_text = false,
                b"p" if self.in_body => self.mapping.finish_paragraph()?,
                b"tc" if self.in_body && self.table_depth > 0 => self.mapping.finish_cell()?,
                b"tr" if self.in_body && self.table_depth > 0 => self.mapping.finish_row()?,
                b"tbl" if self.in_body => {
                    self.table_depth = self
                        .table_depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("DOCX table depth is not balanced"))?;
                }
                b"body" => {
                    if self.mapping.has_open_content() {
                        return Err(invalid("DOCX body ended inside a paragraph or table"));
                    }
                    self.in_body = false;
                }
                _ => {}
            }
        }
        self.xml_depth = self
            .xml_depth
            .checked_sub(1)
            .ok_or_else(|| invalid("DOCX XML end tag has no matching start tag"))?;
        Ok(())
    }

    fn finish(self) -> Result<Vec<TextBlock>, PortError> {
        if !self.root_seen || !self.body_seen || self.in_body || self.xml_depth != 0 {
            return Err(invalid("DOCX document XML is incomplete"));
        }
        if self.mapping.has_open_content() || self.table_depth != 0 {
            return Err(invalid(
                "DOCX document XML ended inside a paragraph or table",
            ));
        }
        Ok(self.mapping.into_blocks())
    }
}

fn is_word_namespace(namespace: &ResolveResult<'_>) -> bool {
    match namespace {
        ResolveResult::Bound(namespace) => {
            namespace.as_ref() == WORD_NAMESPACE || namespace.as_ref() == STRICT_WORD_NAMESPACE
        }
        ResolveResult::Unbound | ResolveResult::Unknown(_) => false,
    }
}

fn decode_reference(reference: &BytesRef<'_>) -> Result<char, PortError> {
    let name = reference
        .decode()
        .map_err(|error| invalid(format!("decode DOCX XML entity: {error}")))?;
    if reference.is_char_ref() {
        return reference
            .resolve_char_ref()
            .map_err(|error| invalid(format!("resolve DOCX character reference: {error}")))?
            .ok_or_else(|| invalid("invalid DOCX character reference"));
    }
    match name.as_ref() {
        "amp" => Ok('&'),
        "lt" => Ok('<'),
        "gt" => Ok('>'),
        "apos" => Ok('\''),
        "quot" => Ok('"'),
        _ => Err(invalid("DOCX XML contains an undeclared entity")),
    }
}
