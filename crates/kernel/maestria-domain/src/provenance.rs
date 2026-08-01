#![forbid(unsafe_code)]

use crate::types::{ArtifactId, ContentRange, ContentRangeError, EvidenceId};
use sha2::{Digest, Sha256};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize,
)]
pub enum ParseStatus {
    Parsed,
    Unsupported,
    Failed,
    MetadataOnly,
    NeedsOcr,
    Quarantined,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, ::serde::Serialize, ::serde::Deserialize,
)]
pub enum RepresentationKind {
    Raw,
    Retrieval,
    Contextual,
    Summary,
    Visual,
}

#[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub struct ParsedRepresentation {
    pub kind: RepresentationKind,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceSpan {
    TextSpan {
        start_line: usize,
        end_line: usize,
    },
    PdfSpan {
        page: usize,
    },
    PdfRegion {
        page: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

impl SourceSpan {
    /// Builds a one-based, inclusive text line span.
    pub fn text_span(start_line: usize, end_line: usize) -> Result<Self, SourceSpanError> {
        if start_line == 0 {
            return Err(SourceSpanError::TextSpanStartMustBePositive);
        }
        if start_line > end_line {
            return Err(SourceSpanError::TextSpanStartAfterEnd {
                start_line,
                end_line,
            });
        }
        Ok(Self::TextSpan {
            start_line,
            end_line,
        })
    }

    /// Builds a one-based PDF page span.
    pub fn pdf_span(page: usize) -> Result<Self, SourceSpanError> {
        if page == 0 {
            return Err(SourceSpanError::PdfPageMustBePositive);
        }
        Ok(Self::PdfSpan { page })
    }

    /// Builds an immutable PDF page region with positive dimensions.
    pub fn pdf_region(
        page: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, SourceSpanError> {
        if page == 0 {
            return Err(SourceSpanError::PdfRegionPageMustBePositive);
        }
        if width == 0 || height == 0 {
            return Err(SourceSpanError::PdfRegionWidthOrHeightZero { width, height });
        }
        Ok(Self::PdfRegion {
            page,
            x,
            y,
            width,
            height,
        })
    }
}

/// Failure while building a validated [`SourceSpan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSpanError {
    TextSpanStartMustBePositive,
    TextSpanStartAfterEnd { start_line: usize, end_line: usize },
    PdfPageMustBePositive,
    PdfRegionPageMustBePositive,
    PdfRegionWidthOrHeightZero { width: u32, height: u32 },
}

impl std::fmt::Display for SourceSpanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextSpanStartMustBePositive => {
                write!(f, "text span start line must be positive")
            }
            Self::TextSpanStartAfterEnd {
                start_line,
                end_line,
            } => write!(
                f,
                "text span start line {start_line} must not exceed end line {end_line}"
            ),
            Self::PdfPageMustBePositive => write!(f, "PDF span page must be positive"),
            Self::PdfRegionPageMustBePositive => {
                write!(f, "PDF region page must be positive")
            }
            Self::PdfRegionWidthOrHeightZero { width, height } => write!(
                f,
                "PDF region width ({width}) and height ({height}) must be positive"
            ),
        }
    }
}

impl std::error::Error for SourceSpanError {}

/// Deterministically produces a content-addressed hash string.
///
/// Returns a `"sha256:<hex>"` string suitable for identifying byte content
/// without requiring the full bytes. The output is stable across all hosts
/// and processes.
pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex_digest(&digest))
}
/// Returns a stable artifact identity for an externally fetched source.
pub fn web_artifact_id_for(url: &str, content_hash: &str) -> ArtifactId {
    let mut hasher = Sha256::new();
    hasher.update(b"maestria:web-artifact\0");
    hasher.update(url.as_bytes());
    hasher.update([0]);
    hasher.update(content_hash.as_bytes());
    let digest = hasher.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    ArtifactId::new(u64::from_be_bytes(id_bytes))
}

/// Returns a stable evidence identity for a fetched web artifact.
pub fn web_evidence_id_for(artifact_id: ArtifactId) -> EvidenceId {
    let mut hasher = Sha256::new();
    hasher.update(b"maestria:web-evidence\0");
    hasher.update(artifact_id.value().to_be_bytes());
    let digest = hasher.finalize();
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[..8]);
    EvidenceId::new(u64::from_be_bytes(id_bytes))
}

pub fn evidence_id_for(artifact_id: ArtifactId, order: u32) -> EvidenceId {
    EvidenceId::new(
        artifact_id
            .value()
            .wrapping_mul(1_000_003)
            .wrapping_add(u64::from(order))
            .wrapping_add(500_001),
    )
}

pub fn line_range_for_chunk(
    source: &str,
    chunk: &str,
    search_start: &mut usize,
) -> Result<ContentRange, ContentRangeError> {
    let found = source
        .get(*search_start..)
        .and_then(|tail| tail.find(chunk).map(|offset| *search_start + offset))
        .or_else(|| source.find(chunk));
    let (start_line, end_line) = match found {
        Some(start) => {
            let end = start.saturating_add(chunk.len());
            *search_start = end;
            let start_line = line_number_at(source, start);
            (
                start_line,
                line_number_at(source, end.saturating_sub(1)).max(start_line),
            )
        }
        None => {
            let start_line = line_number_at(source, *search_start);
            let line_count = chunk.lines().count().max(1);
            (
                start_line,
                start_line.saturating_add(line_count).saturating_sub(1),
            )
        }
    };
    ContentRange::new(start_line, end_line)
}

pub fn excerpt_for(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(240).collect()
}

fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn line_number_at(text: &str, byte_index: usize) -> usize {
    let capped = byte_index.min(text.len());
    text[..capped].bytes().filter(|byte| *byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_span_constructors_enforce_invariants() {
        assert_eq!(
            SourceSpan::text_span(1, 1),
            Ok(SourceSpan::TextSpan {
                start_line: 1,
                end_line: 1
            })
        );
        assert_eq!(
            SourceSpan::text_span(0, 1),
            Err(SourceSpanError::TextSpanStartMustBePositive)
        );
        assert_eq!(
            SourceSpan::text_span(5, 3),
            Err(SourceSpanError::TextSpanStartAfterEnd {
                start_line: 5,
                end_line: 3
            })
        );
        assert_eq!(SourceSpan::pdf_span(1), Ok(SourceSpan::PdfSpan { page: 1 }));
        assert_eq!(
            SourceSpan::pdf_span(0),
            Err(SourceSpanError::PdfPageMustBePositive)
        );
        assert_eq!(
            SourceSpan::pdf_region(2, 1, 2, 3, 4),
            Ok(SourceSpan::PdfRegion {
                page: 2,
                x: 1,
                y: 2,
                width: 3,
                height: 4
            })
        );
        assert_eq!(
            SourceSpan::pdf_region(0, 1, 2, 3, 4),
            Err(SourceSpanError::PdfRegionPageMustBePositive)
        );
        assert_eq!(
            SourceSpan::pdf_region(2, 1, 2, 0, 4),
            Err(SourceSpanError::PdfRegionWidthOrHeightZero {
                width: 0,
                height: 4
            })
        );
    }
}

#[cfg(test)]
mod content_range_tests {
    use super::line_range_for_chunk;
    use crate::entities::{ContentRange, ContentRangeError};

    #[test]
    fn content_range_constructors_enforce_ordering() -> Result<(), Box<dyn std::error::Error>> {
        assert!(ContentRange::new(0, 0).is_ok());
        assert!(ContentRange::new(0, 1).is_ok());
        assert!(ContentRange::new(1, 1).is_ok());
        assert_eq!(
            ContentRange::new(2, 1),
            Err(ContentRangeError::StartAfterEnd { start: 2, end: 1 })
        );
        let decoded: ContentRange = serde_json::from_str(r#"{"start":1,"end":4}"#)?;
        assert_eq!(decoded.start(), 1);
        assert_eq!(decoded.end(), 4);
        assert!(serde_json::from_str::<ContentRange>(r#"{"start":4,"end":1}"#).is_err());
        Ok(())
    }

    #[test]
    fn line_range_for_chunk_is_ordered_and_advances() -> Result<(), Box<dyn std::error::Error>> {
        let source = "alpha\nbeta\ngamma";
        let mut search_start = 0;
        let first = line_range_for_chunk(source, "beta", &mut search_start)?;
        assert_eq!((first.start(), first.end()), (2, 2));
        assert_eq!(&source[search_start..], "\ngamma");
        let last = line_range_for_chunk(source, "absent", &mut search_start)?;
        assert!(last.start() >= 1 && last.start() <= last.end());
        Ok(())
    }
}
