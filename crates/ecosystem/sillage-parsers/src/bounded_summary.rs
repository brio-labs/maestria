#![forbid(unsafe_code)]

use std::ops::Range;
use std::path::Path;

use sillage_domain::{ArtifactId, CreateCardInput};
use sillage_ports::{BoundedParseError, ParseBudget, ParsedChunk, PortError};

use crate::bounded_markdown;
use crate::bounded_output::{copy_bytes_range_bounded, copy_string_bounded};
use crate::chunking::card_id_for;
use crate::tree_builder::domain_source_span;

pub(crate) fn summary_card_for_bounded(
    artifact_id: ArtifactId,
    path: &Path,
    chunks: &[ParsedChunk],
    budget: &mut ParseBudget<'_>,
) -> Result<CreateCardInput, BoundedParseError> {
    let first_chunk = chunks.first().ok_or_else(|| {
        PortError::invalid_input(
            "build parsed artifact card",
            "parsed artifact has no card evidence span",
        )
    })?;
    let title_range = first_non_empty_line(&first_chunk.text, budget)?;
    let title_source = match title_range {
        Some(range) => clean_title_range(first_chunk.text.as_bytes(), range, budget)?,
        None => None,
    };
    let title = match title_source {
        Some(title) => copy_bytes_range_bounded(first_chunk.text.as_bytes(), title, budget)?,
        None => bounded_file_name_title(path, budget)?,
    };
    let unit = if chunks.len() == 1 { "chunk" } else { "chunks" };
    let body = summary_body_bounded(chunks.len(), unit, path, budget)?;
    Ok(CreateCardInput {
        card_id: card_id_for(artifact_id),
        artifact_id,
        node_id: first_chunk.node_id,
        source_span: domain_source_span(&first_chunk.source_span)?,
        title,
        body,
        security: None,
    })
}

fn bounded_file_name_title(
    path: &Path,
    budget: &mut ParseBudget<'_>,
) -> Result<String, BoundedParseError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let ranges = crate::bounded_path::path_ranges(path, budget)?;
    let Some(range) = ranges.file_name else {
        return copy_string_bounded("artifact", budget);
    };
    if !path_name_is_utf8(bytes, &range, budget)? {
        return copy_string_bounded("artifact", budget);
    }
    copy_bytes_range_bounded(bytes, range, budget)
}

fn path_name_is_utf8(
    bytes: &[u8],
    range: &Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<bool, BoundedParseError> {
    let mut offset = range.start;
    while offset < range.end {
        budget.checkpoint()?;
        let (_, next, valid) = next_lossy_path_character(bytes, offset, budget)?;
        if !valid || next > range.end {
            return Ok(false);
        }
        offset = next;
    }
    Ok(true)
}

fn summary_body_bounded(
    chunk_count: usize,
    unit: &str,
    path: &Path,
    budget: &mut ParseBudget<'_>,
) -> Result<String, BoundedParseError> {
    let path_len = lossy_path_display_len(path, budget)?;
    let count_len = decimal_display_len(chunk_count, budget)?;
    let body_len = "Parsed "
        .len()
        .checked_add(count_len)
        .and_then(|len| len.checked_add(" textual ".len()))
        .and_then(|len| len.checked_add(unit.len()))
        .and_then(|len| len.checked_add(" from ".len()))
        .and_then(|len| len.checked_add(path_len))
        .and_then(|len| len.checked_add(1))
        .ok_or_else(|| PortError::invalid_input("admit summary body", "length overflow"))?;
    budget.charge_output(body_len)?;
    let mut output = String::new();
    output
        .try_reserve_exact(body_len)
        .map_err(|error| PortError::internal("reserve bounded summary body", error.to_string()))?;
    append_precharged_string(&mut output, "Parsed ", budget)?;
    append_decimal(&mut output, chunk_count, budget)?;
    append_precharged_string(&mut output, " textual ", budget)?;
    append_precharged_string(&mut output, unit, budget)?;
    append_precharged_string(&mut output, " from ", budget)?;
    append_path_display_bounded(&mut output, path, budget)?;
    append_precharged_string(&mut output, ".", budget)?;
    if output.len() != body_len {
        return Err(PortError::internal(
            "build bounded parser summary",
            "formatted summary length differs from admitted capacity",
        )
        .into());
    }
    Ok(output)
}

fn decimal_display_len(
    mut value: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<usize, BoundedParseError> {
    let mut length = 1;
    while value >= 10 {
        budget.charge_work(1)?;
        value /= 10;
        length += 1;
    }
    budget.charge_work(1)?;
    Ok(length)
}

fn append_decimal(
    output: &mut String,
    value: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    if value >= 10 {
        append_decimal(output, value / 10, budget)?;
    }
    budget.charge_work(1)?;
    output.push(char::from(b'0' + (value % 10) as u8));
    Ok(())
}

fn append_precharged_string(
    output: &mut String,
    text: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        budget.checkpoint()?;
        let end = bounded_markdown::utf8_segment_end(bytes, offset, bytes.len(), budget)?;
        budget.charge_work((end - offset) as u64)?;
        output.push_str(&text[offset..end]);
        offset = end;
    }
    Ok(())
}

fn lossy_path_display_len(
    path: &Path,
    budget: &mut ParseBudget<'_>,
) -> Result<usize, BoundedParseError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut offset = 0;
    let mut length = 0usize;
    while offset < bytes.len() {
        budget.checkpoint()?;
        let (character, next, _) = next_lossy_path_character(bytes, offset, budget)?;
        length = length
            .checked_add(character.len_utf8())
            .ok_or_else(|| PortError::invalid_input("admit summary path", "length overflow"))?;
        offset = next;
    }
    Ok(length)
}

fn append_path_display_bounded(
    output: &mut String,
    path: &Path,
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        budget.checkpoint()?;
        let (character, next, _) = next_lossy_path_character(bytes, offset, budget)?;
        budget.charge_work(character.len_utf8() as u64)?;
        output.push(character);
        offset = next;
    }
    Ok(())
}

fn next_lossy_path_character(
    bytes: &[u8],
    offset: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<(char, usize, bool), BoundedParseError> {
    next_lossy_path_character_with_wtf8(bytes, offset, budget, cfg!(windows))
}

fn next_lossy_path_character_with_wtf8(
    bytes: &[u8],
    offset: usize,
    budget: &mut ParseBudget<'_>,
    allow_wtf8_surrogates: bool,
) -> Result<(char, usize, bool), BoundedParseError> {
    budget.charge_work(1)?;
    let width = bounded_markdown::utf8_width(bytes[offset]);
    if width == 0 {
        return Ok(('\u{fffd}', offset + 1, false));
    }
    if width == 1 {
        return Ok((char::from(bytes[offset]), offset + 1, true));
    }
    let remaining = bytes.len() - offset;
    let scalar_len = width.min(remaining);
    let wtf8_surrogate_candidate =
        allow_wtf8_surrogates && width == 3 && bytes[offset] == 0xed && remaining >= 3;
    if wtf8_surrogate_candidate {
        budget.charge_work(2)?;
        if (0xa0..=0xbf).contains(&bytes[offset + 1]) && (0x80..=0xbf).contains(&bytes[offset + 2])
        {
            return Ok(('\u{fffd}', offset + 3, false));
        }
    } else {
        budget.charge_work((scalar_len - 1) as u64)?;
    }
    let candidate_end = offset + scalar_len;
    match std::str::from_utf8(&bytes[offset..candidate_end]) {
        Ok(scalar) if scalar_len == width => match scalar.chars().next() {
            Some(character) => Ok((character, candidate_end, true)),
            None => Ok(('\u{fffd}', candidate_end, false)),
        },
        Ok(_) => Ok(('\u{fffd}', candidate_end, false)),
        Err(error) => {
            let consumed = match error.error_len() {
                Some(length) => length.min(scalar_len),
                None => scalar_len,
            };
            Ok(('\u{fffd}', offset + consumed.max(1), false))
        }
    }
}

fn first_non_empty_line(
    text: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Option<Range<usize>>, BoundedParseError> {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while let Some(line) = bounded_markdown::next_source_line(bytes, &mut offset, 1, budget)? {
        let trimmed = bounded_markdown::trim_range(bytes, line.text, budget)?;
        if !trimmed.is_empty() {
            return Ok(Some(trimmed));
        }
    }
    Ok(None)
}

fn clean_title_range(
    bytes: &[u8],
    range: Range<usize>,
    budget: &mut ParseBudget<'_>,
) -> Result<Option<Range<usize>>, BoundedParseError> {
    let mut start = range.start;
    while start < range.end {
        budget.charge_work(1)?;
        if bytes[start] != b'#' {
            break;
        }
        start += 1;
    }
    let trimmed = bounded_markdown::trim_range(bytes, start..range.end, budget)?;
    let mut end = trimmed.start;
    for _ in 0..96 {
        if end >= trimmed.end {
            break;
        }
        let (_, next) = bounded_markdown::next_char(bytes, end, trimmed.end, budget)?;
        end = next;
    }
    if end == trimmed.start {
        Ok(None)
    } else {
        Ok(Some(trimmed.start..end))
    }
}
#[cfg(test)]
mod tests {
    use super::next_lossy_path_character_with_wtf8;
    use sillage_ports::{BoundedParseError, ParseBudget, ParseCheckpoint};

    #[test]
    fn windows_wtf8_unpaired_surrogate_uses_one_replacement() -> Result<(), BoundedParseError> {
        let encoded_surrogate = [0xed, 0xa0, 0x80, b'.'];
        let mut continue_parsing = || ParseCheckpoint::Continue;
        let mut budget = ParseBudget::new(3, 0, &mut continue_parsing);
        let decoded =
            next_lossy_path_character_with_wtf8(&encoded_surrogate, 0, &mut budget, true)?;

        assert_eq!(decoded, ('\u{fffd}', 3, false));
        assert_eq!(budget.work_units_used(), 3);
        Ok(())
    }
}
