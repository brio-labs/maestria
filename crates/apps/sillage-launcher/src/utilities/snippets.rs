use std::fmt::Write;

use crate::errors::LauncherError;

use super::MAX_EXPANSION_BYTES;

const TOKENS: [(&str, usize); 4] = [
    ("{query}", 0),
    ("{date:UTC:ISO8601}", 10),
    ("{time:UTC:ISO8601}", 9),
    ("{datetime:UTC:ISO8601}", 20),
];

#[derive(Clone, Copy)]
pub(crate) struct UtcSnapshot {
    pub(crate) year: i32,
    pub(crate) month: i32,
    pub(crate) day: i32,
    pub(crate) hour: i32,
    pub(crate) minute: i32,
    pub(crate) second: i32,
}

pub(super) fn expand(
    template: &str,
    argument: &str,
    clock: impl FnOnce() -> Result<UtcSnapshot, LauncherError>,
) -> Result<String, LauncherError> {
    let mut output_bytes = template.len();
    let mut needs_clock = false;
    let mut remainder = template;
    while let Some((offset, index)) = next_token(remainder) {
        let (token, bytes) = TOKENS[index];
        needs_clock |= index != 0;
        let replacement = if index == 0 { argument.len() } else { bytes };
        output_bytes = output_bytes
            .checked_sub(token.len())
            .and_then(|length| length.checked_add(replacement))
            .ok_or_else(expansion_too_large)?;
        remainder = &remainder[offset + token.len()..];
    }
    if output_bytes > MAX_EXPANSION_BYTES {
        return Err(expansion_too_large());
    }
    let timestamp = if needs_clock {
        let timestamp = clock()?;
        timestamp.validate()?;
        Some(timestamp)
    } else {
        None
    };
    let mut expanded = String::with_capacity(output_bytes);
    let mut remainder = template;
    while let Some((offset, index)) = next_token(remainder) {
        expanded.push_str(&remainder[..offset]);
        if index == 0 {
            expanded.push_str(argument);
        } else if let Some(timestamp) = timestamp {
            timestamp.append(&mut expanded, index)?;
        }
        remainder = &remainder[offset + TOKENS[index].0.len()..];
    }
    expanded.push_str(remainder);
    Ok(expanded)
}

fn next_token(text: &str) -> Option<(usize, usize)> {
    let mut offset = 0;
    while let Some(next) = text[offset..].find('{') {
        offset += next;
        if let Some(index) = TOKENS
            .iter()
            .position(|(token, _)| text[offset..].starts_with(token))
        {
            return Some((offset, index));
        }

        let mut depth = 0usize;
        for (relative, byte) in text[offset..].bytes().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        offset += relative + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if depth != 0 {
            return None;
        }
    }
    None
}

impl UtcSnapshot {
    fn validate(self) -> Result<(), LauncherError> {
        let leap = self.year % 4 == 0 && (self.year % 100 != 0 || self.year % 400 == 0);
        let days = match self.month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => 0,
        };
        if !(1..=9999).contains(&self.year)
            || !(1..=days).contains(&self.day)
            || !(0..=23).contains(&self.hour)
            || !(0..=59).contains(&self.minute)
            || !(0..=59).contains(&self.second)
        {
            return Err(LauncherError::platform_unavailable(
                "UTC clock value is unavailable",
            ));
        }
        Ok(())
    }

    fn append(self, output: &mut String, token: usize) -> Result<(), LauncherError> {
        if token == 1 || token == 3 {
            write!(output, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
                .map_err(|_| expansion_too_large())?;
        }
        if token == 3 {
            output.push('T');
        }
        if token == 2 || token == 3 {
            write!(
                output,
                "{:02}:{:02}:{:02}Z",
                self.hour, self.minute, self.second
            )
            .map_err(|_| expansion_too_large())?;
        }
        Ok(())
    }
}

fn expansion_too_large() -> LauncherError {
    LauncherError::invalid_request("Utility expansion exceeds the 64 KiB limit")
}
