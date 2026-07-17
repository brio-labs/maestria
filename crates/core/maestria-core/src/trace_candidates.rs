use maestria_domain::{
    ArtifactVersionId, ContentRange, Evidence, EvidenceCandidate, EvidenceSpan,
    FreshnessRequirement, FreshnessStatus, RetrievalReason, RetrievalScoreSet, SourceLocation,
    SourceSpan, TrustLabel,
};

use crate::types::SourceGroundedSearchHit;

fn freshness_status(
    evidence: &Evidence,
    requirement: &FreshnessRequirement,
    current_tick: u64,
) -> FreshnessStatus {
    if matches!(requirement, FreshnessRequirement::Any) {
        return FreshnessStatus::Unknown;
    }
    let maestria_domain::EvidenceKind::WebSnapshot { fetched_at, .. } = &evidence.kind else {
        return FreshnessStatus::Unknown;
    };
    let fetched_tick = fetched_at.value();
    if fetched_tick > current_tick {
        return FreshnessStatus::Unknown;
    }
    let age = current_tick.saturating_sub(fetched_tick);
    let allowed_age = match requirement {
        FreshnessRequirement::Realtime => 3,
        FreshnessRequirement::MaximumAgeDays(days) => u64::from(*days),
        FreshnessRequirement::Any => 0,
    };
    if age <= allowed_age {
        FreshnessStatus::UpToDate
    } else {
        FreshnessStatus::Stale
    }
}

pub(super) fn evidence_candidate_from_hit(
    hit: SourceGroundedSearchHit,
    reason: &RetrievalReason,
    semantic: bool,
    freshness_requirement: &FreshnessRequirement,
    current_tick: u64,
) -> Option<EvidenceCandidate> {
    let (location, range) = match &hit.evidence.kind {
        maestria_domain::EvidenceKind::FileSpan { path, range, .. } => {
            let (start_line, end_line) = match hit.chunk.source_span {
                SourceSpan::TextSpan {
                    start_line,
                    end_line,
                } => (start_line as u32, end_line as u32),
                SourceSpan::PdfSpan { .. } => return None,
            };
            (
                SourceLocation::File {
                    path: path.clone(),
                    start_line,
                    end_line,
                },
                *range,
            )
        }
        maestria_domain::EvidenceKind::PdfSpan {
            page_start,
            page_end,
            ..
        } => (
            SourceLocation::Page {
                page_start: *page_start,
                page_end: *page_end,
            },
            ContentRange { start: 0, end: 1 },
        ),
        maestria_domain::EvidenceKind::WebSnapshot { url, .. } => (
            SourceLocation::Symbol {
                path: url.clone(),
                qualified_name: "web_snapshot".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        ),
        maestria_domain::EvidenceKind::CommandOutput {
            harness_run,
            stream,
            ..
        } => (
            SourceLocation::Symbol {
                path: format!("harness:{}", harness_run.value()),
                qualified_name: format!("command_output:{stream:?}"),
            },
            ContentRange { start: 0, end: 1 },
        ),
        maestria_domain::EvidenceKind::TestResult {
            harness_run,
            status,
            ..
        } => (
            SourceLocation::Symbol {
                path: format!("harness:{}", harness_run.value()),
                qualified_name: format!("test_result:{status:?}"),
            },
            ContentRange { start: 0, end: 1 },
        ),
        maestria_domain::EvidenceKind::Diff { harness_run, .. } => (
            SourceLocation::Symbol {
                path: format!("harness:{}", harness_run.value()),
                qualified_name: "diff".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        ),
        maestria_domain::EvidenceKind::Validation { report_id } => (
            SourceLocation::Symbol {
                path: format!("report:{}", report_id.value()),
                qualified_name: "validation".to_string(),
            },
            ContentRange { start: 0, end: 1 },
        ),
    };
    let source_span = EvidenceSpan::new(Some(hit.chunk.node_id), location, range).ok()?;
    let security = hit.artifact.security.taint_from(&hit.evidence.security);
    let trust = match (&security.trust_zone, &security.integrity) {
        (
            maestria_domain::TrustZone::System | maestria_domain::TrustZone::Verified,
            maestria_domain::IntegrityState::Verified,
        ) => TrustLabel::Verified,
        _ => TrustLabel::Unverified,
    };
    Some(EvidenceCandidate {
        evidence_id: hit.evidence.id,
        // Legacy artifacts have no separate version identity; preserve the
        // artifact identity rather than inventing a hash-based version.
        artifact_version: ArtifactVersionId::new(hit.artifact.id.value()),
        source_span,
        scores: RetrievalScoreSet {
            bm25: if semantic { 0 } else { hit.score },
            semantic_similarity: if semantic { hit.score } else { 0 },
        },
        trust,
        freshness: freshness_status(&hit.evidence, freshness_requirement, current_tick),
        duplicate_cluster: None,
        reasons: vec![reason.clone()],
        coverage_keys: Vec::new(),
    })
}
