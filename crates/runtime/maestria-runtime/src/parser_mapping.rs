use maestria_domain::{
    ParseStatus, ParsedRepresentation, RepresentationKind, SourceSpan as DomainSourceSpan,
    SourceSpanError,
};
use maestria_ports::{RepresentationKind as PortRepresentationKind, SourceSpan};

pub(crate) fn domain_parse_status(status: maestria_ports::ParseStatus) -> ParseStatus {
    match status {
        maestria_ports::ParseStatus::Parsed => ParseStatus::Parsed,
        maestria_ports::ParseStatus::Unsupported => ParseStatus::Unsupported,
        maestria_ports::ParseStatus::Failed => ParseStatus::Failed,
        maestria_ports::ParseStatus::MetadataOnly => ParseStatus::MetadataOnly,
        maestria_ports::ParseStatus::NeedsOcr => ParseStatus::NeedsOcr,
        maestria_ports::ParseStatus::Quarantined => ParseStatus::Quarantined,
    }
}

fn domain_representation_kind(kind: PortRepresentationKind) -> RepresentationKind {
    match kind {
        PortRepresentationKind::Raw => RepresentationKind::Raw,
        PortRepresentationKind::Retrieval => RepresentationKind::Retrieval,
        PortRepresentationKind::Contextual => RepresentationKind::Contextual,
        PortRepresentationKind::Summary => RepresentationKind::Summary,
        PortRepresentationKind::Visual => RepresentationKind::Visual,
    }
}

pub(crate) fn domain_source_span(span: &SourceSpan) -> Result<DomainSourceSpan, SourceSpanError> {
    match span {
        SourceSpan::TextSpan {
            start_line,
            end_line,
        } => DomainSourceSpan::text_span(*start_line, *end_line),
        SourceSpan::PdfSpan { page } => DomainSourceSpan::pdf_span(*page),
        SourceSpan::PdfRegion {
            page,
            x,
            y,
            width,
            height,
        } => DomainSourceSpan::pdf_region(*page, *x, *y, *width, *height),
    }
}

pub(crate) fn domain_representation(
    representation: &maestria_ports::ParsedRepresentation,
) -> ParsedRepresentation {
    ParsedRepresentation {
        kind: domain_representation_kind(representation.kind),
        content: representation.content.clone(),
    }
}
