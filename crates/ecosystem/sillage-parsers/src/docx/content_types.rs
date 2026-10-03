use quick_xml::{events::Event, reader::Reader};

use super::{CONTENT_TYPES_MAIN_DOCUMENT, DOCUMENT_PART_NAME, MAX_DOCX_XML_EVENTS, invalid};

pub(super) fn validate_content_types(bytes: &[u8]) -> Result<(), sillage_ports::PortError> {
    let _text = std::str::from_utf8(bytes)
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
    let mut reader = Reader::from_reader(bytes);
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
    Ok(())
}
