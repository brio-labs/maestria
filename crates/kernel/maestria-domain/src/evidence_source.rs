use crate::entities::{OutputStream, TestStatus};
use crate::ids::{BlobId, LogicalTick};
use crate::provenance::content_hash;
use crate::search::ContentHash;
use std::fmt;

/// An immutable locator and validated byte identity for a retrieved snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRef {
    blob_id: BlobId,
    content_hash: ContentHash,
}

impl SnapshotRef {
    pub fn new(blob_id: BlobId, content_hash: ContentHash) -> Self {
        Self {
            blob_id,
            content_hash,
        }
    }

    pub const fn blob_id(&self) -> BlobId {
        self.blob_id
    }

    pub fn content_hash(&self) -> &ContentHash {
        &self.content_hash
    }
}

/// A one-based, inclusive line interval in a text snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineRange {
    start: usize,
    end: usize,
}

impl LineRange {
    pub fn new(start: usize, end: usize) -> Result<Self, LineRangeError> {
        if start == 0 {
            return Err(LineRangeError::StartMustBePositive);
        }
        if start > end {
            return Err(LineRangeError::StartAfterEnd { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn start(&self) -> usize {
        self.start
    }

    pub const fn end(&self) -> usize {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineRangeError {
    StartMustBePositive,
    StartAfterEnd { start: usize, end: usize },
}

impl fmt::Display for LineRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartMustBePositive => write!(f, "line range start must be at least one"),
            Self::StartAfterEnd { start, end } => {
                write!(f, "line range start {start} must not exceed end {end}")
            }
        }
    }
}

impl std::error::Error for LineRangeError {}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebEvidenceMetadata {
    pub published_at: Option<String>,
    pub updated_at: Option<String>,
    pub effective_at: Option<String>,
    pub accessed_at: Option<String>,
    pub content_type: Option<String>,
    pub primary_source: bool,
    pub is_dynamic: bool,
    pub is_paywalled: bool,
}

/// Evidence sources which carry text or an immutable binary snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceKind {
    FileSpan {
        path: String,
        range: LineRange,
        snapshot: SnapshotRef,
    },
    PdfSpan {
        snapshot: SnapshotRef,
        page_start: u32,
        page_end: u32,
    },
    PdfRegion {
        snapshot: SnapshotRef,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    WebSnapshot {
        url: String,
        snapshot: SnapshotRef,
        fetched_at: LogicalTick,
        metadata: WebEvidenceMetadata,
    },
    CommandOutput {
        harness_run: crate::ids::HarnessRunId,
        stream: OutputStream,
        blob: BlobId,
    },
    TestResult {
        harness_run: crate::ids::HarnessRunId,
        status: TestStatus,
        log: BlobId,
    },
    Diff {
        harness_run: crate::ids::HarnessRunId,
        patch_blob: BlobId,
    },
    Validation {
        report_id: crate::ids::ValidationReportId,
    },
}

/// Failure while proving that retrieved bytes are the exact binary snapshot
/// referenced by evidence metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotVerificationError {
    EmptySnapshot,
    HashMismatch {
        expected: ContentHash,
        actual: String,
    },
}

impl fmt::Display for SnapshotVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySnapshot => write!(f, "snapshot must not be empty"),
            Self::HashMismatch { expected, actual } => {
                write!(
                    f,
                    "snapshot hash mismatch: expected {}, got {actual}",
                    expected.as_str()
                )
            }
        }
    }
}

impl std::error::Error for SnapshotVerificationError {}

pub fn verify_snapshot_bytes(
    snapshot: &SnapshotRef,
    retrieved_bytes: &[u8],
) -> Result<(), SnapshotVerificationError> {
    if retrieved_bytes.is_empty() {
        return Err(SnapshotVerificationError::EmptySnapshot);
    }
    let actual_hash = content_hash(retrieved_bytes);
    if actual_hash != snapshot.content_hash().as_str() {
        return Err(SnapshotVerificationError::HashMismatch {
            expected: snapshot.content_hash().clone(),
            actual: actual_hash,
        });
    }
    Ok(())
}

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

    fn snapshot(bytes: &[u8]) -> Result<SnapshotRef, Box<dyn std::error::Error>> {
        Ok(SnapshotRef::new(
            BlobId::new(7),
            ContentHash::new(content_hash(bytes))?,
        ))
    }

    #[test]
    fn line_range_rejects_zero_and_reversed_bounds() {
        assert_eq!(
            LineRange::new(0, 1),
            Err(LineRangeError::StartMustBePositive)
        );
        assert_eq!(
            LineRange::new(3, 2),
            Err(LineRangeError::StartAfterEnd { start: 3, end: 2 })
        );
    }

    #[test]
    fn snapshot_ref_accepts_only_validated_hashes() -> Result<(), Box<dyn std::error::Error>> {
        assert!(ContentHash::new("not-a-sha256-digest".to_string()).is_err());
        let hash = ContentHash::new("sha256:".to_owned() + &"a".repeat(64))?;
        let snapshot = SnapshotRef::new(BlobId::new(11), hash.clone());
        assert_eq!(snapshot.blob_id(), BlobId::new(11));
        assert_eq!(snapshot.content_hash(), &hash);
        Ok(())
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
    #[test]
    fn binary_snapshot_verifier_rejects_empty_and_mismatched_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let expected = b"pdf bytes";
        assert!(verify_snapshot_bytes(&snapshot(expected)?, expected).is_ok());
        assert_eq!(
            verify_snapshot_bytes(&snapshot(expected)?, &[]),
            Err(SnapshotVerificationError::EmptySnapshot)
        );
        assert!(matches!(
            verify_snapshot_bytes(&snapshot(expected)?, b"tampered"),
            Err(SnapshotVerificationError::HashMismatch { .. })
        ));
        Ok(())
    }
}
