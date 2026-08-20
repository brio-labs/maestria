use maestria_domain::{ParseStatus, ParsedRepresentation, RepresentationKind, SourceSpan};
use serde::{Deserialize, Serialize};

crate::stored_enum! {
    #[derive(Default)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StoredParseStatus <=> ParseStatus {
        #[default]
        Parsed,
        Unsupported,
        Failed,
        MetadataOnly,
        NeedsOcr,
        Quarantined,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum StoredSourceSpan {
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
            SourceSpan::PdfRegion {
                page,
                x,
                y,
                width,
                height,
            } => Self::PdfRegion {
                page,
                x,
                y,
                width,
                height,
            },
        }
    }
}

impl TryFrom<StoredSourceSpan> for SourceSpan {
    type Error = maestria_ports::PortError;

    fn try_from(span: StoredSourceSpan) -> Result<Self, Self::Error> {
        match span {
            StoredSourceSpan::TextSpan {
                start_line,
                end_line,
            } => SourceSpan::text_span(start_line, end_line).map_err(|error| {
                maestria_ports::PortError::InvalidInputContext {
                    context: "decode stored source span",
                    source: error.to_string(),
                }
            }),
            StoredSourceSpan::PdfSpan { page } => SourceSpan::pdf_span(page).map_err(|error| {
                maestria_ports::PortError::InvalidInputContext {
                    context: "decode stored source span",
                    source: error.to_string(),
                }
            }),
            StoredSourceSpan::PdfRegion {
                page,
                x,
                y,
                width,
                height,
            } => SourceSpan::pdf_region(page, x, y, width, height).map_err(|error| {
                maestria_ports::PortError::InvalidInputContext {
                    context: "decode stored source span",
                    source: error.to_string(),
                }
            }),
        }
    }
}

crate::stored_enum! {
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StoredRepresentationKind <=> RepresentationKind {
        Raw,
        Retrieval,
        Contextual,
        Summary,
        Visual,
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct StoredParsedRepresentation {
    pub kind: StoredRepresentationKind,
    pub content: String,
}

impl StoredParsedRepresentation {
    pub(crate) fn from_domain(rep: &ParsedRepresentation) -> Self {
        Self {
            kind: StoredRepresentationKind::from_domain(rep.kind),
            content: rep.content.clone(),
        }
    }

    pub(crate) fn try_into_domain(self) -> Result<ParsedRepresentation, maestria_ports::PortError> {
        Ok(ParsedRepresentation {
            kind: self.kind.try_into_domain()?,
            content: self.content,
        })
    }
}

impl From<ParsedRepresentation> for StoredParsedRepresentation {
    fn from(rep: ParsedRepresentation) -> Self {
        Self::from_domain(&rep)
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
