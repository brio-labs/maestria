use maestria_domain::{
    ArtifactVersionId, ContentRange, EvidenceId, EvidenceKind, EvidenceSpan, SourceLocation,
};

use crate::types::SourceGroundedSearchHit;

pub(crate) fn candidate_provenance_matches_hit(
    evidence_id: EvidenceId,
    artifact_version: ArtifactVersionId,
    source_span: &EvidenceSpan,
    hit: &SourceGroundedSearchHit,
) -> bool {
    evidence_id == hit.evidence.id
        && artifact_version.value() == hit.artifact.id.value()
        && source_span.node_id() == Some(hit.chunk.node_id)
        && source_kind_matches_hit(&hit.evidence.kind, source_span, hit)
}

fn source_kind_matches_hit(
    kind: &EvidenceKind,
    source_span: &EvidenceSpan,
    hit: &SourceGroundedSearchHit,
) -> bool {
    match kind {
        EvidenceKind::FileSpan { path, range, .. } => {
            file_span_matches(path, *range, source_span, hit)
        }
        EvidenceKind::PdfSpan {
            page_start,
            page_end,
            ..
        } => pdf_span_matches(*page_start, *page_end, source_span, hit),
        EvidenceKind::PdfRegion {
            page,
            x,
            y,
            width,
            height,
            ..
        } => pdf_region_matches(*page, *x, *y, *width, *height, source_span, hit),
        EvidenceKind::WebSnapshot { url, .. } => symbol_matches(source_span, url, "web_snapshot"),
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            ..
        } => symbol_matches(
            source_span,
            &format!("harness:{}", harness_run.value()),
            &format!("command_output:{stream:?}"),
        ),
        EvidenceKind::TestResult {
            harness_run,
            status,
            ..
        } => symbol_matches(
            source_span,
            &format!("harness:{}", harness_run.value()),
            &format!("test_result:{status:?}"),
        ),
        EvidenceKind::Diff { harness_run, .. } => symbol_matches(
            source_span,
            &format!("harness:{}", harness_run.value()),
            "diff",
        ),
        EvidenceKind::Validation { report_id } => symbol_matches(
            source_span,
            &format!("report:{}", report_id.value()),
            "validation",
        ),
    }
}

fn file_span_matches(
    path: &str,
    range: ContentRange,
    source_span: &EvidenceSpan,
    hit: &SourceGroundedSearchHit,
) -> bool {
    let SourceLocation::File {
        path: candidate_path,
        start_line,
        end_line,
    } = source_span.location()
    else {
        return false;
    };
    path == candidate_path
        && range == source_span.range()
        && matches!(
            hit.chunk.source_span,
            maestria_domain::SourceSpan::TextSpan {
                start_line: hit_start,
                end_line: hit_end,
            } if usize::try_from(*start_line).ok() == Some(hit_start)
                && usize::try_from(*end_line).ok() == Some(hit_end)
        )
}

fn pdf_span_matches(
    page_start: u32,
    page_end: u32,
    source_span: &EvidenceSpan,
    hit: &SourceGroundedSearchHit,
) -> bool {
    let SourceLocation::Page {
        page_start: candidate_start,
        page_end: candidate_end,
    } = source_span.location()
    else {
        return false;
    };
    page_start == *candidate_start
        && page_end == *candidate_end
        && matches!(
            hit.chunk.source_span,
            maestria_domain::SourceSpan::PdfSpan { page }
                if usize::try_from(page_start).ok().is_some_and(|start| page >= start)
                    && usize::try_from(page_end).ok().is_some_and(|end| page <= end)
        )
}
fn pdf_region_matches(
    page: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    source_span: &EvidenceSpan,
    hit: &SourceGroundedSearchHit,
) -> bool {
    let SourceLocation::Region {
        page: candidate_page,
        x: candidate_x,
        y: candidate_y,
        width: candidate_width,
        height: candidate_height,
    } = source_span.location()
    else {
        return false;
    };
    page == *candidate_page
        && x == *candidate_x
        && y == *candidate_y
        && width == *candidate_width
        && height == *candidate_height
        && matches!(
            hit.chunk.source_span,
            maestria_domain::SourceSpan::PdfRegion {
                page: hit_page,
                x: hit_x,
                y: hit_y,
                width: hit_width,
                height: hit_height,
            } if usize::try_from(page).ok() == Some(hit_page)
                && x == hit_x
                && y == hit_y
                && width == hit_width
                && height == hit_height
        )
}

fn symbol_matches(source_span: &EvidenceSpan, path: &str, qualified_name: &str) -> bool {
    matches!(
        source_span.location(),
        SourceLocation::Symbol {
            path: candidate_path,
            qualified_name: candidate_name,
        } if candidate_path == path
            && candidate_name == qualified_name
            && source_span.range() == ContentRange { start: 0, end: 1 }
    )
}
