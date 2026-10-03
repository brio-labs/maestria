#[derive(Debug, Clone, Copy)]
pub(super) struct InvalidText;

pub(super) fn decode_bounded(
    mut bytes: Vec<u8>,
    max_bytes: usize,
    already_truncated: bool,
) -> Result<(String, bool), InvalidText> {
    let truncated = already_truncated || bytes.len() > max_bytes;
    bytes.truncate(max_bytes);
    if let Err(error) = std::str::from_utf8(&bytes) {
        if error.error_len().is_none() && truncated {
            bytes.truncate(error.valid_up_to());
        } else {
            return Err(InvalidText);
        }
    }
    let text = String::from_utf8(bytes).map_err(|_| InvalidText)?;
    Ok((text, truncated))
}

pub(super) fn truncate_utf16(text: &str, max_units: usize) -> String {
    let mut units = 0;
    let mut end = 0;
    for (index, character) in text.char_indices() {
        let width = character.len_utf16();
        if units + width > max_units {
            break;
        }
        units += width;
        end = index + character.len_utf8();
    }
    text[..end].to_owned()
}
