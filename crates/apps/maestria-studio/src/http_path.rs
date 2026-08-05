use anyhow::{Context, Result, anyhow};

pub(super) fn decode_path_segment(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(anyhow!("invalid percent-encoded source key"));
        }
        let high = hex_digit(bytes[index + 1])
            .ok_or_else(|| anyhow!("invalid percent-encoded source key"))?;
        let low = hex_digit(bytes[index + 2])
            .ok_or_else(|| anyhow!("invalid percent-encoded source key"))?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).context("source key is not valid UTF-8")
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::decode_path_segment;

    #[test]
    fn decodes_path_bearing_source_key() {
        assert_eq!(
            decode_path_segment("%2Fworkspace%2Fnotes%2Falpha%20one.md")
                .ok()
                .as_deref(),
            Some("/workspace/notes/alpha one.md")
        );
    }

    #[test]
    fn rejects_malformed_path_encoding() {
        assert!(decode_path_segment("%2Fworkspace%2F%ZZ").is_err());
        assert!(decode_path_segment("%2Fworkspace%2F%").is_err());
    }
}
