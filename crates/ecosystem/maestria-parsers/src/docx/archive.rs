use std::io::Cursor;

use zip::ZipArchive;

use super::content_types::validate_content_types;
use super::mapping::TextBlock;
use super::paragraphs::extract_paragraphs;
use super::part_reader::read_part;
use super::{
    MAX_DOCX_ARCHIVE_BYTES, MAX_DOCX_CONTENT_TYPES_BYTES, MAX_DOCX_DOCUMENT_XML_BYTES,
    MAX_DOCX_ENTRIES, MAX_DOCX_UNCOMPRESSED_BYTES, invalid,
};

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;

#[derive(Clone, Copy)]
pub(super) struct ArchiveFacts {
    entries: usize,
    document_xml_size: usize,
    content_types_size: usize,
}

struct CentralDirectory {
    entries: usize,
    start: usize,
    end: usize,
}

#[derive(Default)]
struct RequiredParts {
    document_xml: Option<u64>,
    content_types: Option<u64>,
}

struct EntryFacts<'a> {
    name: &'a [u8],
    flags: u16,
    method: u16,
    compressed_size: u64,
    record_end: usize,
    uncompressed_size: u64,
}

#[derive(Clone, Copy)]
enum RequiredPart {
    DocumentXml,
    ContentTypes,
}

pub(super) fn extract_document_blocks(
    bytes: &[u8],
) -> Result<Vec<TextBlock>, maestria_ports::PortError> {
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

fn inspect_archive(bytes: &[u8]) -> Result<ArchiveFacts, maestria_ports::PortError> {
    let directory = inspect_directory(bytes)?;
    let parts = inspect_directory_entries(bytes, &directory)?;
    let document_xml_size = required_part_size(
        parts.document_xml,
        "DOCX main document part is missing",
        "DOCX document size is invalid",
        "document",
        "extraction",
        MAX_DOCX_DOCUMENT_XML_BYTES,
    )?;
    let content_types_size = required_part_size(
        parts.content_types,
        "DOCX content-types part is missing",
        "DOCX content-types size is invalid",
        "content-types",
        "validation",
        MAX_DOCX_CONTENT_TYPES_BYTES,
    )?;
    Ok(ArchiveFacts {
        entries: directory.entries,
        document_xml_size,
        content_types_size,
    })
}

fn inspect_directory(bytes: &[u8]) -> Result<CentralDirectory, maestria_ports::PortError> {
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
    let (entry_count, directory_size, directory_offset) = read_directory_footer(bytes, eocd)?;
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
    let end = eocd;
    let start = end
        .checked_sub(directory_size)
        .ok_or_else(|| invalid("DOCX ZIP directory extends before the archive"))?;
    if directory_offset > start {
        return Err(invalid("DOCX ZIP directory offset is inconsistent"));
    }
    Ok(CentralDirectory {
        entries,
        start,
        end,
    })
}

fn read_directory_footer(
    bytes: &[u8],
    eocd: usize,
) -> Result<(u16, u32, u32), maestria_ports::PortError> {
    let disk_number = read_footer_u16(bytes, eocd + 4)?;
    let directory_disk = read_footer_u16(bytes, eocd + 6)?;
    let disk_entries = read_footer_u16(bytes, eocd + 8)?;
    let entry_count = read_footer_u16(bytes, eocd + 10)?;
    let directory_size = read_footer_u32(bytes, eocd + 12)?;
    let directory_offset = read_footer_u32(bytes, eocd + 16)?;
    if disk_number != 0 || directory_disk != 0 || disk_entries != entry_count {
        return Err(invalid("multi-disk DOCX ZIP archives are unsupported"));
    }
    if entry_count == u16::MAX || directory_size == u32::MAX || directory_offset == u32::MAX {
        return Err(invalid(
            "ZIP64 DOCX archives are unsupported by the bounded parser",
        ));
    }
    Ok((entry_count, directory_size, directory_offset))
}

fn inspect_directory_entries(
    bytes: &[u8],
    directory: &CentralDirectory,
) -> Result<RequiredParts, maestria_ports::PortError> {
    let mut cursor = directory.start;
    let mut total_uncompressed = 0_u64;
    let mut parts = RequiredParts::default();
    for _ in 0..directory.entries {
        let entry = inspect_entry(bytes, cursor, directory.end)?;
        total_uncompressed = total_uncompressed
            .checked_add(entry.uncompressed_size)
            .ok_or_else(|| invalid("DOCX uncompressed size overflow"))?;
        if total_uncompressed > MAX_DOCX_UNCOMPRESSED_BYTES {
            return Err(invalid(format!(
                "DOCX total uncompressed size exceeds {MAX_DOCX_UNCOMPRESSED_BYTES} bytes"
            )));
        }
        let required_part =
            validate_xml_part(entry.name, entry.flags, entry.method, entry.compressed_size)?;
        record_required_part(&mut parts, required_part, entry.uncompressed_size)?;
        cursor = entry.record_end;
    }
    if cursor != directory.end {
        return Err(invalid(
            "DOCX ZIP central directory size does not match its entries",
        ));
    }
    Ok(parts)
}

fn inspect_entry(
    bytes: &[u8],
    cursor: usize,
    directory_end: usize,
) -> Result<EntryFacts<'_>, maestria_ports::PortError> {
    if read_u32(bytes, cursor) != Some(CENTRAL_HEADER_SIGNATURE) {
        return Err(invalid("DOCX ZIP central directory entry is malformed"));
    }
    let flags = entry_u16(bytes, cursor + 8)?;
    let method = entry_u16(bytes, cursor + 10)?;
    let compressed_size = entry_u32(bytes, cursor + 20)?;
    let uncompressed_size = entry_u32(bytes, cursor + 24)?;
    let name_len = usize::from(entry_u16(bytes, cursor + 28)?);
    let extra_len = usize::from(entry_u16(bytes, cursor + 30)?);
    let comment_len = usize::from(entry_u16(bytes, cursor + 32)?);
    let disk_start = entry_u16(bytes, cursor + 34)?;
    let local_header_offset = entry_u32(bytes, cursor + 42)?;
    validate_entry_format(
        compressed_size,
        uncompressed_size,
        local_header_offset,
        disk_start,
    )?;
    let (name, extra_start, record_end) = entry_record_bounds(
        bytes,
        cursor,
        directory_end,
        name_len,
        extra_len,
        comment_len,
    )?;
    validate_extra_fields(bytes, extra_start, extra_len)?;
    Ok(EntryFacts {
        name,
        flags,
        method,
        compressed_size: u64::from(compressed_size),
        record_end,
        uncompressed_size: u64::from(uncompressed_size),
    })
}

fn validate_entry_format(
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
    disk_start: u16,
) -> Result<(), maestria_ports::PortError> {
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
    Ok(())
}

fn entry_record_bounds(
    bytes: &[u8],
    cursor: usize,
    directory_end: usize,
    name_len: usize,
    extra_len: usize,
    comment_len: usize,
) -> Result<(&[u8], usize, usize), maestria_ports::PortError> {
    let fixed_end = cursor
        .checked_add(46)
        .ok_or_else(|| invalid("DOCX ZIP directory length overflow"))?;
    let name_end = fixed_end
        .checked_add(name_len)
        .ok_or_else(|| invalid("DOCX ZIP entry name length overflow"))?;
    let extra_start = name_end;
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
        .get(fixed_end..name_end)
        .ok_or_else(|| invalid("DOCX ZIP entry name is truncated"))?;
    Ok((name, extra_start, record_end))
}

fn validate_xml_part(
    name: &[u8],
    flags: u16,
    method: u16,
    compressed_size: u64,
) -> Result<Option<RequiredPart>, maestria_ports::PortError> {
    let required_part = if name == b"word/document.xml" {
        Some(RequiredPart::DocumentXml)
    } else if name == b"[Content_Types].xml" {
        Some(RequiredPart::ContentTypes)
    } else {
        return Ok(None);
    };
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
    Ok(required_part)
}

fn record_required_part(
    parts: &mut RequiredParts,
    required_part: Option<RequiredPart>,
    uncompressed_size: u64,
) -> Result<(), maestria_ports::PortError> {
    let Some(required_part) = required_part else {
        return Ok(());
    };
    let destination = match required_part {
        RequiredPart::DocumentXml => &mut parts.document_xml,
        RequiredPart::ContentTypes => &mut parts.content_types,
    };
    if destination.is_some() {
        return Err(invalid("DOCX ZIP contains duplicate required XML parts"));
    }
    *destination = Some(uncompressed_size);
    Ok(())
}

fn required_part_size(
    size: Option<u64>,
    missing_message: &str,
    conversion_message: &str,
    part_name: &str,
    action: &str,
    maximum_size: usize,
) -> Result<usize, maestria_ports::PortError> {
    let size = match size {
        Some(size) => size,
        None => return Err(invalid(missing_message)),
    };
    let size =
        usize::try_from(size).map_err(|error| invalid(format!("{conversion_message}: {error}")))?;
    if size == 0 || size > maximum_size {
        return Err(invalid(format!(
            "DOCX {part_name} part exceeds the {maximum_size} byte {action} limit"
        )));
    }
    Ok(size)
}

fn entry_u16(bytes: &[u8], offset: usize) -> Result<u16, maestria_ports::PortError> {
    read_u16(bytes, offset).ok_or_else(|| invalid("truncated ZIP entry"))
}

fn entry_u32(bytes: &[u8], offset: usize) -> Result<u32, maestria_ports::PortError> {
    read_u32(bytes, offset).ok_or_else(|| invalid("truncated ZIP entry"))
}

fn read_footer_u16(bytes: &[u8], offset: usize) -> Result<u16, maestria_ports::PortError> {
    read_u16(bytes, offset).ok_or_else(|| invalid("truncated DOCX ZIP footer"))
}

fn read_footer_u32(bytes: &[u8], offset: usize) -> Result<u32, maestria_ports::PortError> {
    read_u32(bytes, offset).ok_or_else(|| invalid("truncated DOCX ZIP footer"))
}

fn validate_extra_fields(
    bytes: &[u8],
    start: usize,
    length: usize,
) -> Result<(), maestria_ports::PortError> {
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

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    Some(u16::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    Some(u32::from_le_bytes(bytes.get(offset..end)?.try_into().ok()?))
}
