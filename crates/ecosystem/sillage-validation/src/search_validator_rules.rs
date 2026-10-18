use std::collections::BTreeSet;

use sillage_domain::{CorpusScope, EvidenceKind, ScopeId, Sensitivity, TrustZone};

use crate::types::SearchValidationContext;

pub(super) fn outcome_evidence_ids(
    search: &SearchValidationContext<'_>,
) -> BTreeSet<sillage_domain::EvidenceId> {
    search.candidate_ids().collect()
}

pub(super) fn is_primary_source(evidence: &sillage_domain::Evidence) -> bool {
    match &evidence.kind {
        EvidenceKind::WebSnapshot { metadata, .. } => metadata.primary_source,
        _ => true,
    }
}

pub(super) fn denied_candidate_count(
    search: &SearchValidationContext<'_>,
    scope: &CorpusScope,
    effective_scopes: Option<&[ScopeId]>,
    required_trust: Option<&TrustZone>,
    maximum_sensitivity: Option<&Sensitivity>,
    policy_allows_unscoped: bool,
) -> usize {
    let plan_scopes = match scope {
        CorpusScope::Restricted(scopes) => Some(scopes.as_slice()),
        CorpusScope::Global => None,
    };
    let allowed_scopes = effective_scopes.or(plan_scopes);
    search
        .outcome
        .evidence
        .iter()
        .filter(|candidate| {
            let Some(evidence) = search.evidence_record(candidate.evidence_id()) else {
                return false;
            };
            let Some(artifact) = search.artifact_record(evidence.artifact_id) else {
                return true;
            };
            let security = artifact.security.taint_from(&evidence.security);
            let scope_denied = allowed_scopes.is_some_and(|scopes| {
                security
                    .scope_id
                    .as_ref()
                    .is_some_and(|scope_id| !scopes.contains(scope_id))
            });
            let trust_denied = required_trust
                .is_some_and(|required| !security_zone_satisfies(&security.trust_zone, required));
            let sensitivity_denied = maximum_sensitivity
                .is_some_and(|maximum| security.sensitivity.level() > maximum.level());
            let unscoped_denied =
                security.scope_id.is_none() && !policy_allows_unscoped && allowed_scopes.is_some();
            scope_denied
                || trust_denied
                || sensitivity_denied
                || unscoped_denied
                || !security.retrieval_allowed()
                || security.prompt_injection_risk
                || !security.poisoning_flags.is_empty()
        })
        .count()
}

pub(super) fn policy_snapshot(
    value: &str,
) -> Result<sillage_domain::RetrievalPolicySnapshot, String> {
    sillage_domain::RetrievalPolicySnapshot::from_canonical(value)
        .map_err(|error| format!("invalid policy snapshot: {error:?}"))
}

/// Whether `actual` meets a required minimum trust zone (System > Verified > Untrusted).
fn security_zone_satisfies(actual: &TrustZone, required: &TrustZone) -> bool {
    matches!(
        (actual, required),
        (TrustZone::System, _)
            | (
                TrustZone::Verified,
                TrustZone::Verified | TrustZone::Untrusted
            )
            | (TrustZone::Untrusted, TrustZone::Untrusted)
    )
}

pub(super) fn symbol_span_matches(
    source_span: &sillage_domain::EvidenceSpan,
    path: &str,
    qualified_name: &str,
) -> bool {
    matches!(
        source_span.location(),
        sillage_domain::SourceLocation::Symbol {
            path: candidate_path,
            qualified_name: candidate_name,
            ..
        } if candidate_path == path
            && candidate_name == qualified_name
            && source_span.range().start() == 0
            && source_span.range().end() == 1
    )
}

pub(super) fn span_matches_record(
    candidate: &sillage_domain::EvidenceCandidate,
    evidence: &sillage_domain::Evidence,
) -> bool {
    match (candidate.source_span().location(), &evidence.kind) {
        (
            sillage_domain::SourceLocation::DocxParagraph {
                path,
                start_paragraph,
                end_paragraph,
            },
            sillage_domain::EvidenceKind::DocxParagraphSpan {
                path: evidence_path,
                range,
                ..
            },
        ) => {
            path == evidence_path
                && *start_paragraph == range.start()
                && *end_paragraph == range.end()
                && usize::try_from(*start_paragraph).ok()
                    == Some(candidate.source_span().range().start())
                && usize::try_from(*end_paragraph).ok()
                    == Some(candidate.source_span().range().end())
        }
        (
            sillage_domain::SourceLocation::File { path, .. },
            sillage_domain::EvidenceKind::FileSpan {
                path: evidence_path,
                range,
                ..
            },
        ) => {
            path == evidence_path
                && candidate.source_span().range().start() == range.start()
                && candidate.source_span().range().end() == range.end()
        }
        (
            sillage_domain::SourceLocation::File { path, .. },
            sillage_domain::EvidenceKind::PdfSpan { .. },
        ) => path == "document.pdf",
        (
            sillage_domain::SourceLocation::File { path, .. },
            sillage_domain::EvidenceKind::PdfRegion { .. },
        ) => {
            candidate.source_span().range().start() == 0
                && candidate.source_span().range().end() == 1
                && path == "document.pdf"
        }
        (
            sillage_domain::SourceLocation::Symbol { .. },
            sillage_domain::EvidenceKind::PdfSpan { .. }
            | sillage_domain::EvidenceKind::PdfRegion { .. }
            | sillage_domain::EvidenceKind::FileSpan { .. },
        ) => false,
        (
            sillage_domain::SourceLocation::File { path, .. },
            sillage_domain::EvidenceKind::WebSnapshot { .. },
        ) => path.starts_with("http://") || path.starts_with("https://"),
        (
            sillage_domain::SourceLocation::Symbol { .. },
            sillage_domain::EvidenceKind::WebSnapshot { .. },
        ) => false,
        (
            sillage_domain::SourceLocation::Symbol { .. },
            sillage_domain::EvidenceKind::CommandOutput { harness_run, .. },
        ) => symbol_span_matches(
            candidate.source_span(),
            &format!("run:{}", harness_run.value()),
            "command",
        ),
        (
            sillage_domain::SourceLocation::Symbol { .. },
            sillage_domain::EvidenceKind::TestResult { harness_run, .. },
        ) => symbol_span_matches(
            candidate.source_span(),
            &format!("run:{}", harness_run.value()),
            "test",
        ),
        (
            sillage_domain::SourceLocation::Symbol { .. },
            sillage_domain::EvidenceKind::Diff { harness_run, .. },
        ) => symbol_span_matches(
            candidate.source_span(),
            &format!("run:{}", harness_run.value()),
            "diff",
        ),
        (
            sillage_domain::SourceLocation::Symbol { .. },
            sillage_domain::EvidenceKind::Validation { report_id },
        ) => symbol_span_matches(
            candidate.source_span(),
            &format!("report:{}", report_id.value()),
            "validation",
        ),
        _ => false,
    }
}

pub(super) fn expected_trust(evidence: &sillage_domain::Evidence) -> sillage_domain::TrustLabel {
    match (&evidence.security.trust_zone, &evidence.security.integrity) {
        (
            sillage_domain::TrustZone::System | sillage_domain::TrustZone::Verified,
            sillage_domain::IntegrityState::Verified,
        ) => sillage_domain::TrustLabel::Verified,
        _ => sillage_domain::TrustLabel::Unverified,
    }
}

pub(super) fn candidate_matches_record(
    candidate: &sillage_domain::EvidenceCandidate,
    evidence: &sillage_domain::Evidence,
) -> bool {
    candidate.artifact_version().value() == evidence.artifact_id.value()
        && span_matches_record(candidate, evidence)
        && candidate.trust() == expected_trust(evidence)
}
