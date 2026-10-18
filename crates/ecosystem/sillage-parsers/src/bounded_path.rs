#![forbid(unsafe_code)]

use std::ops::Range;
use std::path::Path;

use sillage_ports::{BoundedParseError, ParseBudget};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PathRanges {
    pub(crate) file_name: Option<Range<usize>>,
    pub(crate) extension: Option<Range<usize>>,
}

pub(crate) fn path_ranges(
    path: &Path,
    budget: &mut ParseBudget<'_>,
) -> Result<PathRanges, BoundedParseError> {
    path_ranges_for_platform(path.as_os_str().as_encoded_bytes(), cfg!(windows), budget)
}

fn path_ranges_for_platform(
    bytes: &[u8],
    windows: bool,
    budget: &mut ParseBudget<'_>,
) -> Result<PathRanges, BoundedParseError> {
    let (mut component_start, verbatim) = path_prefix_scan_start(bytes, windows, budget)?;
    if windows
        && component_start > 0
        && charged_byte(bytes, component_start, budget)?.is_some_and(is_windows_separator)
    {
        component_start += 1;
    }

    let scan_start = component_start;
    let mut ranges = PathRanges::default();
    let mut component_start = scan_start;
    let mut last_dot = None;
    for (offset, byte) in bytes[scan_start..].iter().enumerate() {
        let index = scan_start + offset;
        budget.charge_work(1)?;
        let byte = *byte;
        if path_separator(byte, windows, verbatim) {
            record_component(
                bytes,
                component_start..index,
                last_dot,
                verbatim,
                &mut ranges,
                budget,
            )?;
            component_start = index + 1;
            last_dot = None;
        } else if byte == b'.' {
            last_dot = Some(index);
        }
    }
    record_component(
        bytes,
        component_start..bytes.len(),
        last_dot,
        verbatim,
        &mut ranges,
        budget,
    )?;
    Ok(ranges)
}

fn path_prefix_scan_start(
    bytes: &[u8],
    windows: bool,
    budget: &mut ParseBudget<'_>,
) -> Result<(usize, bool), BoundedParseError> {
    if windows {
        windows_prefix_scan_start(bytes, budget)
    } else {
        Ok((0, false))
    }
}

fn windows_prefix_scan_start(
    bytes: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<(usize, bool), BoundedParseError> {
    if bytes.len() < 2 {
        return Ok((0, false));
    }
    let Some(first) = charged_byte(bytes, 0, budget)? else {
        return Ok((0, false));
    };
    let Some(second) = charged_byte(bytes, 1, budget)? else {
        return Ok((0, false));
    };
    if is_windows_separator(first) && is_windows_separator(second) {
        let third = charged_byte(bytes, 2, budget)?;
        let fourth = charged_byte(bytes, 3, budget)?;
        if third == Some(b'?') && fourth == Some(b'\\') && first != b'/' && second != b'/' {
            return windows_verbatim_prefix_scan_start(bytes, budget);
        }
        if third == Some(b'.') && fourth.is_some_and(is_windows_separator) {
            let (device_end, _) = component_end(bytes, 4, true, false, budget)?;
            return Ok((device_end, false));
        }

        let (server_end, server_has_separator) = component_end(bytes, 2, true, false, budget)?;
        if server_end == 2 || !server_has_separator {
            return Ok((0, false));
        }
        let share_start = server_end + 1;
        let (share_end, _) = component_end(bytes, share_start, true, false, budget)?;
        if share_end == share_start {
            return Ok((0, false));
        }
        return Ok((share_end, false));
    }

    if first.is_ascii_alphabetic() && second == b':' {
        Ok((2, false))
    } else {
        Ok((0, false))
    }
}

fn windows_verbatim_prefix_scan_start(
    bytes: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<(usize, bool), BoundedParseError> {
    if windows_prefix_matches(bytes, 4, b"UNC\\", budget)? {
        let (server_end, server_has_separator) = component_end(bytes, 8, true, true, budget)?;
        let share_start = if server_has_separator {
            server_end + 1
        } else {
            server_end
        };
        let (share_end, _) = component_end(bytes, share_start, true, true, budget)?;
        let prefix_end = if share_end > share_start {
            share_end
        } else {
            server_end
        };
        return Ok((prefix_end, true));
    }

    let drive = charged_byte(bytes, 4, budget)?;
    let separator = charged_byte(bytes, 5, budget)?;
    if drive.is_some_and(|byte| byte.is_ascii_alphabetic()) && separator == Some(b':') {
        let next = charged_byte(bytes, 6, budget)?;
        if next.is_none() || next.is_some_and(is_windows_separator) {
            return Ok((6, true));
        }
    }

    let (prefix_end, _) = component_end(bytes, 4, true, true, budget)?;
    Ok((prefix_end, true))
}

fn windows_prefix_matches(
    bytes: &[u8],
    start: usize,
    expected: &[u8],
    budget: &mut ParseBudget<'_>,
) -> Result<bool, BoundedParseError> {
    for (offset, expected_byte) in expected.iter().copied().enumerate() {
        let Some(byte) = charged_byte(bytes, start + offset, budget)? else {
            return Ok(false);
        };
        let normalized = if byte == b'/' { b'\\' } else { byte };
        if normalized != expected_byte {
            return Ok(false);
        }
    }
    Ok(true)
}

fn component_end(
    bytes: &[u8],
    start: usize,
    windows: bool,
    verbatim: bool,
    budget: &mut ParseBudget<'_>,
) -> Result<(usize, bool), BoundedParseError> {
    let mut end = start;
    while end < bytes.len() {
        budget.charge_work(1)?;
        if path_separator(bytes[end], windows, verbatim) {
            return Ok((end, true));
        }
        end += 1;
    }
    Ok((end, false))
}

fn charged_byte(
    bytes: &[u8],
    index: usize,
    budget: &mut ParseBudget<'_>,
) -> Result<Option<u8>, BoundedParseError> {
    if index >= bytes.len() {
        return Ok(None);
    }
    budget.charge_work(1)?;
    Ok(Some(bytes[index]))
}

fn path_separator(byte: u8, windows: bool, verbatim: bool) -> bool {
    if !windows {
        byte == b'/'
    } else if verbatim {
        byte == b'\\'
    } else {
        is_windows_separator(byte)
    }
}

fn is_windows_separator(byte: u8) -> bool {
    matches!(byte, b'/' | b'\\')
}

fn record_component(
    bytes: &[u8],
    range: Range<usize>,
    last_dot: Option<usize>,
    verbatim: bool,
    ranges: &mut PathRanges,
    budget: &mut ParseBudget<'_>,
) -> Result<(), BoundedParseError> {
    match range.len() {
        0 => {}
        1 => {
            budget.charge_work(1)?;
            if bytes[range.start] == b'.' {
                if verbatim {
                    ranges.file_name = None;
                    ranges.extension = None;
                }
            } else {
                record_normal_component(range, last_dot, ranges);
            }
        }
        2 => {
            budget.charge_work(2)?;
            if bytes[range.start] == b'.' && bytes[range.start + 1] == b'.' {
                ranges.file_name = None;
                ranges.extension = None;
            } else {
                record_normal_component(range, last_dot, ranges);
            }
        }
        _ => record_normal_component(range, last_dot, ranges),
    }
    Ok(())
}

fn record_normal_component(range: Range<usize>, last_dot: Option<usize>, ranges: &mut PathRanges) {
    ranges.extension = last_dot
        .filter(|dot| *dot > range.start)
        .map(|dot| dot + 1..range.end);
    ranges.file_name = Some(range);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sillage_ports::{ParseBudgetFailure, ParseCheckpoint};

    #[test]
    fn path_ranges_match_host_path_metadata() -> Result<(), BoundedParseError> {
        for source in [
            "guide.md/",
            "guide.md/.",
            "guide.md/..",
            "nested/../guide.md",
            "...",
            "hidden.",
            ".hidden",
            "/",
            ".",
            "..",
        ] {
            let path = Path::new(source);
            let bytes = path.as_os_str().as_encoded_bytes();
            let mut continue_parsing = || ParseCheckpoint::Continue;
            let mut budget = ParseBudget::new(u64::MAX, usize::MAX, &mut continue_parsing);
            let ranges = path_ranges(path, &mut budget)?;
            let file_name = ranges.file_name.map(|range| &bytes[range]);
            let extension = ranges.extension.map(|range| &bytes[range]);
            assert_eq!(
                file_name,
                path.file_name().map(|name| name.as_encoded_bytes())
            );
            assert_eq!(
                extension,
                path.extension().map(|name| name.as_encoded_bytes())
            );
        }
        Ok(())
    }

    #[test]
    fn windows_prefixes_are_not_filename_components() -> Result<(), BoundedParseError> {
        let cases: &[(&str, Option<&str>, Option<&str>)] = &[
            ("\\\\server\\share", None, None),
            ("\\\\server\\share\\", None, None),
            ("\\\\server\\share\\guide.md", Some("guide.md"), Some("md")),
            ("\\\\?\\C:", None, None),
            ("\\\\?\\C:\\", None, None),
            ("\\\\?\\C:/file.md", Some("file.md"), Some("md")),
            ("\\\\?\\pictures/file.md", None, None),
            ("\\\\?\\C:\\dir\\", Some("dir"), None),
            ("\\\\?\\C:\\dir\\.", None, None),
            ("\\\\?\\C:\\dir\\.\\file.md", Some("file.md"), Some("md")),
            ("C:\\dir\\.", Some("dir"), None),
            ("\\\\?\\pictures\\.\\kittens", Some("kittens"), None),
            ("\\\\?\\C:\\dir\\file.md", Some("file.md"), Some("md")),
            ("\\\\?\\UNC\\server\\share", None, None),
            (
                "\\\\?\\UNC\\server\\share\\file.md",
                Some("file.md"),
                Some("md"),
            ),
            ("\\\\.\\COM42", None, None),
            ("\\\\.\\COM42\\file.md", Some("file.md"), Some("md")),
            ("\\\\?\\pictures", None, None),
            ("\\\\?\\pictures\\kittens", Some("kittens"), None),
            ("C:", None, None),
            ("C:\\", None, None),
            ("C:relative.md", Some("relative.md"), Some("md")),
            ("\\\\server", Some("server"), None),
        ];

        for &(source, expected_name, expected_extension) in cases {
            let bytes = source.as_bytes();
            let mut continue_parsing = || ParseCheckpoint::Continue;
            let mut budget = ParseBudget::new(u64::MAX, usize::MAX, &mut continue_parsing);
            let ranges = path_ranges_for_platform(bytes, true, &mut budget)?;
            let file_name = ranges.file_name.map(|range| &bytes[range]);
            let extension = ranges.extension.map(|range| &bytes[range]);
            assert_eq!(file_name, expected_name.map(str::as_bytes));
            assert_eq!(extension, expected_extension.map(str::as_bytes));
        }
        Ok(())
    }

    #[test]
    fn filename_scan_observes_cancellation_during_real_traversal() {
        let mut bytes = vec![b'x'; 4096];
        bytes.extend_from_slice(b".md");
        let mut calls = 0;
        let result = {
            let mut cancel = || {
                calls += 1;
                ParseCheckpoint::Cancelled
            };
            let mut budget = ParseBudget::new(10_000, 0, &mut cancel);
            path_ranges_for_platform(&bytes, false, &mut budget)
        };
        assert!(matches!(
            result,
            Err(BoundedParseError::Budget(ParseBudgetFailure::Cancelled))
        ));
        assert_eq!(calls, 1);
    }

    #[test]
    fn windows_prefix_scan_observes_cancellation_during_real_traversal() {
        let mut bytes = b"\\\\".to_vec();
        bytes.extend(std::iter::repeat_n(b's', 4096));
        bytes.extend_from_slice(b"\\share");
        let mut calls = 0;
        let result = {
            let mut cancel = || {
                calls += 1;
                ParseCheckpoint::Cancelled
            };
            let mut budget = ParseBudget::new(10_000, 0, &mut cancel);
            path_ranges_for_platform(&bytes, true, &mut budget)
        };
        assert!(matches!(
            result,
            Err(BoundedParseError::Budget(ParseBudgetFailure::Cancelled))
        ));
        assert_eq!(calls, 1);
    }
}
