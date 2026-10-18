#![forbid(unsafe_code)]

use std::ops::Range;

use sillage_ports::{BoundedParseError, ParseBudget, PortError, SourceSpan};

const UTF8_SEGMENT_BYTES: usize = 128;

pub(crate) struct TextChunkRange {
    pub(crate) text: Range<usize>,
    pub(crate) raw: Range<usize>,
    pub(crate) source_span: SourceSpan,
}

pub(crate) struct SourceLine {
    pub(crate) text: Range<usize>,
    pub(crate) start: usize,
    pub(crate) number: usize,
}

pub(crate) fn chunk_ranges(
    bytes: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<Vec<TextChunkRange>, BoundedParseError> {
    if bytes.is_empty() {
        return Err(PortError::invalid_input("decode file bytes", "input file is empty").into());
    }
    let mut offset = 0;
    let mut number = 1;
    while let Some(line) = next_source_line(bytes, &mut offset, number, budget)? {
        if is_heading(bytes, &line.text, budget)? {
            return heading_ranges(bytes, budget);
        }
        number += 1;
    }
    paragraph_ranges(bytes, budget)
}

pub(crate) fn next_source_line(
    bytes: &[u8],
    offset: &mut usize,
    number: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<Option<SourceLine>, BoundedParseError> {
    if *offset >= bytes.len() {
        return Ok(None);
    }
    let start = *offset;
    let mut end = start;
    while end < bytes.len() {
        budget.charge_work(1)?;
        if bytes[end] == b'\n' {
            break;
        }
        end += 1;
    }
    let content_end = if end > start && end < bytes.len() && bytes[end - 1] == b'\r' {
        end - 1
    } else {
        end
    };
    validate_utf8_range(bytes, start..content_end, budget)?;
    *offset = if end < bytes.len() { end + 1 } else { end };
    Ok(Some(SourceLine {
        text: start..content_end,
        start,
        number,
    }))
}

fn heading_ranges(
    bytes: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<Vec<TextChunkRange>, BoundedParseError> {
    let mut chunks = Vec::new();
    let mut offset = 0;
    let mut number = 1;
    let mut section_start: Option<(usize, usize)> = None;
    while let Some(line) = next_source_line(bytes, &mut offset, number, budget)? {
        if is_heading(bytes, &line.text, budget)? {
            if let Some((start, start_line)) = section_start {
                push_range(
                    &mut chunks,
                    bytes,
                    start..line.start,
                    start..line.start,
                    SourceSpan::TextSpan {
                        start_line,
                        end_line: line.number - 1,
                    },
                    budget,
                )?;
            } else if line.start > 0 {
                push_range(
                    &mut chunks,
                    bytes,
                    0..line.start,
                    0..line.start,
                    SourceSpan::TextSpan {
                        start_line: 1,
                        end_line: line.number - 1,
                    },
                    budget,
                )?;
            }
            section_start = Some((line.start, line.number));
        }
        number += 1;
    }
    if let Some((start, start_line)) = section_start {
        push_range(
            &mut chunks,
            bytes,
            start..bytes.len(),
            start..bytes.len(),
            SourceSpan::TextSpan {
                start_line,
                end_line: number - 1,
            },
            budget,
        )?;
    }
    Ok(chunks)
}

fn paragraph_ranges(
    bytes: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<Vec<TextChunkRange>, BoundedParseError> {
    let mut chunks = Vec::new();
    let mut offset = 0;
    let mut number = 1;
    let mut paragraph_start: Option<(usize, usize)> = None;
    let mut last_content_end = 0;
    while let Some(line) = next_source_line(bytes, &mut offset, number, budget)? {
        if is_whitespace(bytes, &line.text, budget)? {
            if let Some((start, start_line)) = paragraph_start.take() {
                push_range(
                    &mut chunks,
                    bytes,
                    start..last_content_end,
                    start..line.start,
                    SourceSpan::TextSpan {
                        start_line,
                        end_line: line.number - 1,
                    },
                    budget,
                )?;
            }
        } else {
            if paragraph_start.is_none() {
                paragraph_start = Some((line.start, line.number));
            }
            last_content_end = line.text.end;
        }
        number += 1;
    }
    if let Some((start, start_line)) = paragraph_start {
        push_range(
            &mut chunks,
            bytes,
            start..last_content_end,
            start..bytes.len(),
            SourceSpan::TextSpan {
                start_line,
                end_line: number - 1,
            },
            budget,
        )?;
    }
    Ok(chunks)
}

fn is_heading(
    bytes: &[u8],
    line: &Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<bool, BoundedParseError> {
    let mut cursor = line.start;
    while cursor < line.end {
        let (character, next) = next_char(bytes, cursor, line.end, budget)?;
        if !character.is_whitespace() {
            break;
        }
        cursor = next;
    }
    let mut marker_len = 0;
    while cursor < line.end {
        budget.charge_work(1)?;
        if bytes[cursor] != b'#' {
            break;
        }
        marker_len += 1;
        cursor += 1;
        if marker_len > 6 {
            return Ok(false);
        }
    }
    if marker_len == 0 {
        return Ok(false);
    }
    if cursor == line.end {
        return Ok(true);
    }
    let (character, _) = next_char(bytes, cursor, line.end, budget)?;
    Ok(matches!(character, ' ' | '\t' | '\r' | '\n'))
}

fn is_whitespace(
    bytes: &[u8],
    range: &Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<bool, BoundedParseError> {
    let mut cursor = range.start;
    while cursor < range.end {
        let (character, next) = next_char(bytes, cursor, range.end, budget)?;
        if !character.is_whitespace() {
            return Ok(false);
        }
        cursor = next;
    }
    Ok(true)
}

pub(crate) fn trim_range(
    bytes: &[u8],
    range: Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<Range<usize>, BoundedParseError> {
    let mut start = range.start;
    let mut end = range.end;
    while start < end {
        let (character, next) = next_char(bytes, start, end, budget)?;
        if !character.is_whitespace() {
            break;
        }
        start = next;
    }
    while end > start {
        let (character, previous) = previous_char(bytes, start, end, budget)?;
        if !character.is_whitespace() {
            break;
        }
        end = previous;
    }
    Ok(start..end)
}

pub(crate) fn next_char(
    bytes: &[u8],
    offset: usize,
    end: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<(char, usize), BoundedParseError> {
    if offset >= end {
        return Err(invalid_utf8("unexpected end of UTF-8 range"));
    }
    budget.charge_work(1)?;
    let width = utf8_width(bytes[offset]);
    if width == 0 || width > end - offset {
        return Err(invalid_utf8("invalid UTF-8 leading byte"));
    }
    if width > 1 {
        budget.charge_work((width - 1) as u64)?;
    }
    let next = offset + width;
    let value = std::str::from_utf8(&bytes[offset..next]).map_err(invalid_utf8_error)?;
    match value.chars().next() {
        Some(character) => Ok((character, next)),
        None => Err(invalid_utf8("empty UTF-8 scalar")),
    }
}

fn previous_char(
    bytes: &[u8],
    start: usize,
    end: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<(char, usize), BoundedParseError> {
    let mut scalar_start = end - 1;
    budget.charge_work(1)?;
    let mut byte = bytes[scalar_start];
    while byte & 0b1100_0000 == 0b1000_0000 {
        if scalar_start == start {
            return Err(invalid_utf8("invalid UTF-8 continuation"));
        }
        scalar_start -= 1;
        budget.charge_work(1)?;
        byte = bytes[scalar_start];
    }
    let width = utf8_width(byte);
    if width == 0 || scalar_start + width != end {
        return Err(invalid_utf8("invalid UTF-8 scalar boundary"));
    }
    let value = std::str::from_utf8(&bytes[scalar_start..end]).map_err(invalid_utf8_error)?;
    match value.chars().next() {
        Some(character) => Ok((character, scalar_start)),
        None => Err(invalid_utf8("empty UTF-8 scalar")),
    }
}

pub(crate) fn utf8_width(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}

pub(crate) fn utf8_segment_end(
    bytes: &[u8],
    start: usize,
    end: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<usize, BoundedParseError> {
    let mut segment_end = start.saturating_add(UTF8_SEGMENT_BYTES).min(end);
    let mut backed_up = 0;
    while segment_end < end {
        budget.charge_work(1)?;
        if bytes[segment_end] & 0b1100_0000 != 0b1000_0000 {
            break;
        }
        if backed_up == 3 {
            return Err(invalid_utf8(
                "invalid UTF-8 continuation at segment boundary",
            ));
        }
        segment_end -= 1;
        backed_up += 1;
    }
    if segment_end == start {
        return Err(invalid_utf8("UTF-8 scalar exceeds bounded segment"));
    }
    Ok(segment_end)
}

fn validate_utf8_range(
    bytes: &[u8],
    range: Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    let mut offset = range.start;
    while offset < range.end {
        budget.checkpoint()?;
        let end = utf8_segment_end(bytes, offset, range.end, budget)?;
        budget.charge_work((end - offset) as u64)?;
        std::str::from_utf8(&bytes[offset..end]).map_err(invalid_utf8_error)?;
        offset = end;
    }
    Ok(())
}

fn push_range(
    chunks: &mut Vec<TextChunkRange>,
    bytes: &[u8],
    candidate: Range<usize>,
    raw: Range<usize>,
    source_span: SourceSpan,
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    let text = trim_range(bytes, candidate, budget)?;
    if text.is_empty() {
        return Ok(());
    }
    budget.charge_work(1)?;
    if chunks.len() == chunks.capacity() {
        let next_capacity = if chunks.capacity() == 0 {
            4
        } else {
            chunks.capacity().checked_mul(2).ok_or_else(|| {
                PortError::invalid_input("reserve Markdown ranges", "capacity overflow")
            })?
        };
        let additional = next_capacity - chunks.len();
        let bytes = std::mem::size_of::<TextChunkRange>()
            .checked_mul(additional)
            .ok_or_else(|| PortError::invalid_input("reserve Markdown ranges", "size overflow"))?;
        budget.charge_output(bytes)?;
        chunks.try_reserve_exact(additional).map_err(|error| {
            PortError::internal("reserve Markdown chunk ranges", error.to_string())
        })?;
    }
    chunks.push(TextChunkRange {
        text,
        raw,
        source_span,
    });
    Ok(())
}

fn invalid_utf8(message: &str) -> BoundedParseError {
    PortError::invalid_input("file bytes are not utf8", message).into()
}

fn invalid_utf8_error(error: std::str::Utf8Error) -> BoundedParseError {
    PortError::invalid_input("file bytes are not utf8", error.to_string()).into()
}
