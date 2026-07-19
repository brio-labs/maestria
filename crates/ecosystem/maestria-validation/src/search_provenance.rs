use maestria_domain::{EvidenceKind, SourceLocation, TrustLabel};

use super::search_validators::evaluate_search;
use super::types::{ValidationCheck, ValidationContext, Validator};

fn symbol_span_matches(
    source_span: &maestria_domain::EvidenceSpan,
    path: &str,
    qualified_name: &str,
) -> bool {
    matches!(
        source_span.location(),
        SourceLocation::Symbol {
            path: candidate_path,
            qualified_name: candidate_name,
        } if candidate_path == path
            && candidate_name == qualified_name
            && source_span.range() == maestria_domain::ContentRange { start: 0, end: 1 }
    )
}

fn span_matches_record(
    candidate: &maestria_domain::EvidenceCandidate,
    evidence: &maestria_domain::Evidence,
) -> bool {
    match (&candidate.source_span.location(), &evidence.kind) {
        (
            SourceLocation::File { path, .. },
            EvidenceKind::FileSpan {
                path: evidence_path,
                range,
                ..
            },
        ) => path == evidence_path && candidate.source_span.range() == *range,
        (
            SourceLocation::Page {
                page_start,
                page_end,
            },
            EvidenceKind::PdfSpan {
                page_start: evidence_start,
                page_end: evidence_end,
                ..
            },
        ) => {
            page_start == evidence_start
                && page_end == evidence_end
                && candidate.source_span.range()
                    == maestria_domain::ContentRange { start: 0, end: 1 }
        }
        (
            SourceLocation::Region {
                page,
                x,
                y,
                width,
                height,
            },
            EvidenceKind::PdfRegion {
                page: evidence_page,
                x: evidence_x,
                y: evidence_y,
                width: evidence_width,
                height: evidence_height,
                ..
            },
        ) => {
            page == evidence_page
                && x == evidence_x
                && y == evidence_y
                && width == evidence_width
                && height == evidence_height
                && candidate.source_span.range()
                    == maestria_domain::ContentRange { start: 0, end: 1 }
        }
        (SourceLocation::Symbol { .. }, EvidenceKind::WebSnapshot { url, .. }) => {
            symbol_span_matches(&candidate.source_span, url, "web_snapshot")
        }
        (
            SourceLocation::Symbol { .. },
            EvidenceKind::CommandOutput {
                harness_run,
                stream,
                ..
            },
        ) => symbol_span_matches(
            &candidate.source_span,
            &format!("harness:{}", harness_run.value()),
            &format!("command_output:{stream:?}"),
        ),
        (
            SourceLocation::Symbol { .. },
            EvidenceKind::TestResult {
                harness_run,
                status,
                ..
            },
        ) => symbol_span_matches(
            &candidate.source_span,
            &format!("harness:{}", harness_run.value()),
            &format!("test_result:{status:?}"),
        ),
        (SourceLocation::Symbol { .. }, EvidenceKind::Diff { harness_run, .. }) => {
            symbol_span_matches(
                &candidate.source_span,
                &format!("harness:{}", harness_run.value()),
                "diff",
            )
        }
        (SourceLocation::Symbol { .. }, EvidenceKind::Validation { report_id }) => {
            symbol_span_matches(
                &candidate.source_span,
                &format!("report:{}", report_id.value()),
                "validation",
            )
        }
        _ => false,
    }
}

fn expected_trust(evidence: &maestria_domain::Evidence) -> TrustLabel {
    match (&evidence.security.trust_zone, &evidence.security.integrity) {
        (
            maestria_domain::TrustZone::System | maestria_domain::TrustZone::Verified,
            maestria_domain::IntegrityState::Verified,
        ) => TrustLabel::Verified,
        _ => TrustLabel::Unverified,
    }
}

fn candidate_matches_record(
    candidate: &maestria_domain::EvidenceCandidate,
    evidence: &maestria_domain::Evidence,
) -> bool {
    candidate.artifact_version.value() == evidence.artifact_id.value()
        && span_matches_record(candidate, evidence)
        && candidate.trust == expected_trust(evidence)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CandidateProvenanceValidator;

impl Validator for CandidateProvenanceValidator {
    fn name(&self) -> &str {
        "candidate_provenance"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
            let Some(trace) = search.trace else {
                return Err(
                    "candidate provenance cannot be checked without a SearchTrace".to_string(),
                );
            };
            let invalid_records = search
                .outcome
                .evidence
                .iter()
                .filter(|candidate| {
                    search
                        .evidence_record(candidate.evidence_id)
                        .is_none_or(|evidence| !candidate_matches_record(candidate, evidence))
                })
                .count();
            let trace_matches = trace.matches_evidence(&search.outcome.evidence);
            if invalid_records == 0 && trace_matches {
                Ok(
                    "all candidates resolve to persisted evidence with matching provenance"
                        .to_string(),
                )
            } else {
                Err(format!(
                    "candidate provenance is invalid: {invalid_records} invalid evidence record(s), trace_match={trace_matches}"
                ))
            }
        })
    }
}
