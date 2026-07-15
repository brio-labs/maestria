use maestria_domain::{ParseStatus, ParsedRepresentation, RepresentationKind, SourceSpan};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredParseStatus {
    #[default]
    Parsed,
    Unsupported,
    Failed,
    MetadataOnly,
    NeedsOcr,
    Quarantined,
}

impl From<ParseStatus> for StoredParseStatus {
    fn from(status: ParseStatus) -> Self {
        match status {
            ParseStatus::Parsed => Self::Parsed,
            ParseStatus::Unsupported => Self::Unsupported,
            ParseStatus::Failed => Self::Failed,
            ParseStatus::MetadataOnly => Self::MetadataOnly,
            ParseStatus::NeedsOcr => Self::NeedsOcr,
            ParseStatus::Quarantined => Self::Quarantined,
        }
    }
}

impl From<StoredParseStatus> for ParseStatus {
    fn from(status: StoredParseStatus) -> Self {
        match status {
            StoredParseStatus::Parsed => Self::Parsed,
            StoredParseStatus::Unsupported => Self::Unsupported,
            StoredParseStatus::Failed => Self::Failed,
            StoredParseStatus::MetadataOnly => Self::MetadataOnly,
            StoredParseStatus::NeedsOcr => Self::NeedsOcr,
            StoredParseStatus::Quarantined => Self::Quarantined,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum StoredSourceSpan {
    TextSpan { start_line: usize, end_line: usize },
    PdfSpan { page: usize },
}

impl Default for StoredSourceSpan {
    fn default() -> Self {
        Self::TextSpan {
            start_line: 1,
            end_line: 1,
        }
    }
}

impl From<SourceSpan> for StoredSourceSpan {
    fn from(span: SourceSpan) -> Self {
        match span {
            SourceSpan::TextSpan {
                start_line,
                end_line,
            } => Self::TextSpan {
                start_line,
                end_line,
            },
            SourceSpan::PdfSpan { page } => Self::PdfSpan { page },
        }
    }
}

impl From<StoredSourceSpan> for SourceSpan {
    fn from(span: StoredSourceSpan) -> Self {
        match span {
            StoredSourceSpan::TextSpan {
                start_line,
                end_line,
            } => Self::TextSpan {
                start_line,
                end_line,
            },
            StoredSourceSpan::PdfSpan { page } => Self::PdfSpan { page },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRepresentationKind {
    Raw,
    Retrieval,
    Contextual,
    Summary,
    Visual,
}

impl From<RepresentationKind> for StoredRepresentationKind {
    fn from(kind: RepresentationKind) -> Self {
        match kind {
            RepresentationKind::Raw => Self::Raw,
            RepresentationKind::Retrieval => Self::Retrieval,
            RepresentationKind::Contextual => Self::Contextual,
            RepresentationKind::Summary => Self::Summary,
            RepresentationKind::Visual => Self::Visual,
        }
    }
}

impl From<StoredRepresentationKind> for RepresentationKind {
    fn from(kind: StoredRepresentationKind) -> Self {
        match kind {
            StoredRepresentationKind::Raw => Self::Raw,
            StoredRepresentationKind::Retrieval => Self::Retrieval,
            StoredRepresentationKind::Contextual => Self::Contextual,
            StoredRepresentationKind::Summary => Self::Summary,
            StoredRepresentationKind::Visual => Self::Visual,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct StoredParsedRepresentation {
    pub kind: StoredRepresentationKind,
    pub content: String,
}

impl From<ParsedRepresentation> for StoredParsedRepresentation {
    fn from(rep: ParsedRepresentation) -> Self {
        Self {
            kind: rep.kind.into(),
            content: rep.content,
        }
    }
}

impl From<StoredParsedRepresentation> for ParsedRepresentation {
    fn from(rep: StoredParsedRepresentation) -> Self {
        ParsedRepresentation {
            kind: rep.kind.into(),
            content: rep.content,
        }
    }
}

pub(crate) fn default_status_parsed() -> StoredParseStatus {
    StoredParseStatus::Parsed
}
