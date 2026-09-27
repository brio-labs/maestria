use crate::entities::{OutputStream, TestStatus};
use crate::ids::{BlobId, LogicalTick};
use crate::provenance::content_hash;
use crate::search::ContentHash;
use std::fmt;

#[path = "evidence_source_text.rs"]
mod text;
pub use self::text::{TextSnapshotVerificationError, verify_text_snapshot};

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

/// A one-based, inclusive paragraph interval in a DOCX source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParagraphRange {
    start: u32,
    end: u32,
}

impl ParagraphRange {
    pub fn new(start: u32, end: u32) -> Result<Self, ParagraphRangeError> {
        if start == 0 {
            return Err(ParagraphRangeError::StartMustBePositive);
        }
        if start > end {
            return Err(ParagraphRangeError::StartAfterEnd { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn start(&self) -> u32 {
        self.start
    }

    pub const fn end(&self) -> u32 {
        self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParagraphRangeError {
    StartMustBePositive,
    StartAfterEnd { start: u32, end: u32 },
}

impl fmt::Display for ParagraphRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartMustBePositive => write!(f, "paragraph range start must be at least one"),
            Self::StartAfterEnd { start, end } => {
                write!(f, "paragraph range start {start} must not exceed end {end}")
            }
        }
    }
}

impl std::error::Error for ParagraphRangeError {}

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
    DocxParagraphSpan {
        path: String,
        range: ParagraphRange,
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
    fn paragraph_range_rejects_zero_and_reversed_bounds() {
        assert_eq!(
            ParagraphRange::new(0, 1),
            Err(ParagraphRangeError::StartMustBePositive)
        );
        assert_eq!(
            ParagraphRange::new(3, 2),
            Err(ParagraphRangeError::StartAfterEnd { start: 3, end: 2 })
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
