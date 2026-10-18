use super::{LineRange, SnapshotRef};
use crate::provenance::content_hash;
use crate::search::ContentHash;
use std::fmt;

/// Failure while proving that retrieved bytes are the exact text snapshot
/// referenced by evidence metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextSnapshotVerificationError {
    EmptySnapshot,
    HashMismatch {
        expected: ContentHash,
        actual: String,
    },
    InvalidUtf8,
    RangeOutOfBounds {
        range: LineRange,
        line_count: usize,
    },
    ExcerptNotFound {
        range: Option<LineRange>,
    },
}

impl fmt::Display for TextSnapshotVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySnapshot => write!(f, "text snapshot must not be empty"),
            Self::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "text snapshot hash mismatch: expected {}, got {actual}",
                    expected.as_str()
                )
            }
            Self::InvalidUtf8 => write!(f, "text snapshot is not valid UTF-8"),
            Self::RangeOutOfBounds { range, line_count } => write!(
                f,
                "text snapshot line range {}-{} exceeds {} available lines",
                range.start(),
                range.end(),
                line_count
            ),
            Self::ExcerptNotFound { range: Some(range) } => write!(
                f,
                "excerpt token sequence is absent from selected lines {}-{}",
                range.start(),
                range.end()
            ),
            Self::ExcerptNotFound { range: None } => {
                write!(f, "excerpt token sequence is absent from text snapshot")
            }
        }
    }
}

impl std::error::Error for TextSnapshotVerificationError {}

/// Proves that bytes retrieved for a text evidence source are its exact,
/// strictly UTF-8 snapshot and that the excerpt occurs in the requested lines.
///
/// Token comparison intentionally streams over the selected lines. It accepts
/// equivalent line/word whitespace (including CRLF) without compacting the
/// complete document or allocating a second copy of it.
pub fn verify_text_snapshot(
    snapshot: &SnapshotRef,
    retrieved_bytes: &[u8],
    range: Option<&LineRange>,
    excerpt: &str,
) -> Result<(), TextSnapshotVerificationError> {
    if retrieved_bytes.is_empty() {
        return Err(TextSnapshotVerificationError::EmptySnapshot);
    }

    let actual_hash = content_hash(retrieved_bytes);
    if actual_hash != snapshot.content_hash().as_str() {
        return Err(TextSnapshotVerificationError::HashMismatch {
            expected: snapshot.content_hash().clone(),
            actual: actual_hash,
        });
    }

    let text = std::str::from_utf8(retrieved_bytes)
        .map_err(|_| TextSnapshotVerificationError::InvalidUtf8)?;
    let line_count = text.lines().count();
    let (start, end) = match range {
        Some(range) => {
            if range.end() > line_count {
                return Err(TextSnapshotVerificationError::RangeOutOfBounds {
                    range: *range,
                    line_count,
                });
            }
            (range.start(), range.end())
        }
        None => (1, line_count),
    };

    if token_sequence_in_lines(text, start, end, excerpt) {
        Ok(())
    } else {
        Err(TextSnapshotVerificationError::ExcerptNotFound {
            range: range.copied(),
        })
    }
}

fn token_sequence_in_lines(text: &str, start: usize, end: usize, excerpt: &str) -> bool {
    let mut expected = excerpt.split_whitespace();
    let Some(first_expected) = expected.next() else {
        return true;
    };

    let mut candidates = text
        .lines()
        .skip(start.saturating_sub(1))
        .take(end.saturating_sub(start).saturating_add(1))
        .flat_map(|line| line.split_whitespace());

    while let Some(candidate) = candidates.next() {
        if candidate != first_expected {
            continue;
        }
        let mut remainder = candidates.clone();
        let mut remaining_expected = excerpt.split_whitespace().skip(1);
        if remaining_expected.all(|token| remainder.next() == Some(token)) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::BlobId;

    fn snapshot(bytes: &[u8]) -> Result<SnapshotRef, Box<dyn std::error::Error>> {
        Ok(SnapshotRef::new(
            BlobId::new(7),
            ContentHash::new(content_hash(bytes))?,
        ))
    }

    #[test]
    fn verifier_requires_exact_selected_line_and_strict_utf8()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = "first\r\nsecond café\r\nlast".as_bytes();
        let selected = LineRange::new(2, 2)?;
        assert!(
            verify_text_snapshot(&snapshot(bytes)?, bytes, Some(&selected), "second café").is_ok()
        );
        assert!(matches!(
            verify_text_snapshot(&snapshot(bytes)?, bytes, Some(&selected), "first"),
            Err(TextSnapshotVerificationError::ExcerptNotFound { .. })
        ));

        let invalid = [0xff, 0xfe];
        assert!(matches!(
            verify_text_snapshot(&snapshot(&invalid)?, &invalid, None, ""),
            Err(TextSnapshotVerificationError::InvalidUtf8)
        ));
        Ok(())
    }

    #[test]
    fn verifier_rejects_empty_bytes_hash_mismatch_and_out_of_bounds_range()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = b"one\ntwo";
        assert_eq!(
            verify_text_snapshot(&snapshot(bytes)?, &[], None, ""),
            Err(TextSnapshotVerificationError::EmptySnapshot)
        );
        let wrong = SnapshotRef::new(
            BlobId::new(7),
            ContentHash::new("sha256:".to_owned() + &"0".repeat(64))?,
        );
        assert!(matches!(
            verify_text_snapshot(&wrong, bytes, None, "one"),
            Err(TextSnapshotVerificationError::HashMismatch { .. })
        ));
        let range = LineRange::new(2, 3)?;
        assert!(matches!(
            verify_text_snapshot(&snapshot(bytes)?, bytes, Some(&range), "two"),
            Err(TextSnapshotVerificationError::RangeOutOfBounds { .. })
        ));
        Ok(())
    }
}
