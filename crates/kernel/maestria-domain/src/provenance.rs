#![forbid(unsafe_code)]

use crate::types::{ArtifactId, EvidenceId};
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

impl RepresentationKind {
    /// Stable one-byte tag for digest encoding. Part of the
    /// [`representations_digest`] contract — never renumber.
    pub(crate) fn tag(self) -> u8 {
        match self {
            Self::Raw => 1,
            Self::Retrieval => 2,
            Self::Contextual => 3,
            Self::Summary => 4,
            Self::Visual => 5,
        }
    }
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
const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for &byte in &digest {
        out.push(HEX_CHARS[(byte >> 4) as usize] as char);
        out.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Stable content digest for a representation list.
///
/// Encodes each representation as a kind tag followed by a length-prefixed
/// content byte string, then hashes the result. The encoding is part of the
/// digest contract: it is persisted in events and projection rows, so any
/// change requires a migration. Chunk registration uses the digest as the
/// identity of the representation set — events carry it instead of
/// duplicating chunk text once per representation, and restart recovery
/// compares digests instead of requiring both sides to hold full contents.
pub fn representations_digest(representations: &[ParsedRepresentation]) -> String {
    let mut canonical = Vec::new();
    for representation in representations {
        canonical.push(representation.kind.tag());
        let content = representation.content.as_bytes();
        canonical.extend_from_slice(&(content.len() as u64).to_le_bytes());
        canonical.extend_from_slice(content);
    }
    content_hash(&canonical)
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

pub fn excerpt_for(text: &str) -> String {
    const MAX_EXCERPT_CHARS: usize = 240;
    let mut excerpt = String::new();
    let mut excerpt_chars: usize = 0;
    // Keep token boundaries intact: source verification compares whitespace
    // tokens, so a character-boundary cut would invalidate the evidence.
    for word in text.split_whitespace() {
        let word_chars = word.chars().count();
        let separator_chars = usize::from(!excerpt.is_empty());
        if !excerpt.is_empty()
            && excerpt_chars
                .saturating_add(separator_chars)
                .saturating_add(word_chars)
                > MAX_EXCERPT_CHARS
        {
            break;
        }
        if !excerpt.is_empty() {
            excerpt.push(' ');
            excerpt_chars += 1;
        }
        excerpt.push_str(word);
        excerpt_chars += word_chars;
    }
    excerpt
}

#[cfg(test)]
mod excerpt_tests {
    use super::excerpt_for;
    use crate::{BlobId, ContentHash, LineRange, SnapshotRef, content_hash, verify_text_snapshot};

    #[test]
    fn excerpt_truncates_between_tokens() {
        let text = format!("{} audit record", "word ".repeat(48));
        let excerpt = excerpt_for(&text);

        assert_eq!(excerpt, "word ".repeat(48).trim_end().to_owned());
        assert!(!excerpt.ends_with("audi"));
    }

    #[test]
    fn excerpt_keeps_a_single_oversized_token_intact() {
        let text = "x".repeat(241);

        assert_eq!(excerpt_for(&text), text);
    }

    #[test]
    fn excerpt_verifies_against_the_source_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let source = format!("{} audit record", "word ".repeat(48));
        let snapshot = SnapshotRef::new(
            BlobId::new(1),
            ContentHash::new(content_hash(source.as_bytes()))?,
        );
        let range = LineRange::new(1, 1)?;

        verify_text_snapshot(
            &snapshot,
            source.as_bytes(),
            Some(&range),
            &excerpt_for(&source),
        )?;
        Ok(())
    }
}

pub fn hex_digest(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX_CHARS[(byte >> 4) as usize] as char);
        out.push(HEX_CHARS[(byte & 0x0f) as usize] as char);
    }
    out
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
}
