use std::collections::BTreeSet;

use maestria_domain::{
    EvidenceCandidate, EvidenceCoverage, EvidenceRequirements, FreshnessStatus, SearchPlan,
    SearchStatus, SearchStopReason, SearchTraceDiversity, SearchTraceDiversityCandidate,
    SourceLocation,
};

use crate::types::RankedCandidate;

/// Result of deterministic diversity-aware candidate selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiversitySelection {
    pub candidates: Vec<RankedCandidate>,
    pub trace: SearchTraceDiversity,
    pub coverage: EvidenceCoverage,
    pub status: SearchStatus,
}

struct SelectionSummary<'a> {
    requirements: &'a EvidenceRequirements,
    required_keys: &'a [String],
    covered_keys: &'a BTreeSet<String>,
    selected: &'a [RankedCandidate],
    trace_candidates: Vec<SearchTraceDiversityCandidate>,
    max_results: usize,
    source_count: usize,
    document_count: usize,
    section_count: usize,
    stop_reason: Option<SearchStopReason>,
}

/// Selects bounded, diverse evidence while preserving the incoming rank order.
///
/// Duplicate clusters are hard exclusions. Source, document, section, and coverage
/// requirements are satisfied greedily before low-marginal-gain stopping is allowed.
pub fn select_candidates(ranked: &[RankedCandidate], plan: &SearchPlan) -> DiversitySelection {
    let requirements = &plan.evidence_requirements;
    let max_results = plan.stop_conditions.max_results as usize;
    let required_keys = required_keys(requirements);
    let mut selected = Vec::new();
    let mut trace_candidates = Vec::new();
    let mut covered_keys = BTreeSet::new();
    let mut seen_clusters = BTreeSet::new();
    let mut seen_sources = BTreeSet::new();
    let mut seen_documents = BTreeSet::new();
    let mut seen_sections = BTreeSet::new();
    let mut stop_reason = None;

    for candidate in ranked {
        if selected.len() >= max_results {
            stop_reason = Some(SearchStopReason::ResultsLimit);
            break;
        }

        let evidence = &candidate.candidate;
        if evidence
            .duplicate_cluster
            .is_some_and(|cluster| !seen_clusters.insert(cluster))
        {
            trace_candidates.push(trace_candidate(evidence, candidate.rank, None, 0));
            continue;
        }

        let (source, document, section, marginal_coverage, marginal_gain) = candidate_metrics(
            evidence,
            &covered_keys,
            &seen_sources,
            &seen_documents,
            &seen_sections,
        );

        if requirements_satisfied(
            requirements,
            &required_keys,
            &covered_keys,
            seen_sources.len(),
            seen_documents.len(),
            seen_sections.len(),
        ) && marginal_gain == 0
        {
            trace_candidates.push(trace_candidate(evidence, candidate.rank, None, 0));
            stop_reason = Some(SearchStopReason::LowMarginalGain);
            break;
        }

        seen_sources.insert(source);
        seen_documents.insert(document);
        seen_sections.insert(section);
        covered_keys.extend(evidence.coverage_keys.iter().cloned());
        let selected_rank = selected.len();
        selected.push(candidate.clone());
        trace_candidates.push(trace_candidate(
            evidence,
            candidate.rank,
            Some(selected_rank),
            marginal_coverage,
        ));
    }
    let (stop_reason, coverage, trace) = finalize_selection(SelectionSummary {
        requirements,
        required_keys: &required_keys,
        covered_keys: &covered_keys,
        selected: &selected,
        trace_candidates,
        max_results,
        source_count: seen_sources.len(),
        document_count: seen_documents.len(),
        section_count: seen_sections.len(),
        stop_reason,
    });
    let status = selection_status(&selected, &coverage, requirements, &stop_reason);

    DiversitySelection {
        candidates: selected,
        trace,
        coverage,
        status,
    }
}
fn finalize_selection(
    summary: SelectionSummary<'_>,
) -> (SearchStopReason, EvidenceCoverage, SearchTraceDiversity) {
    let SelectionSummary {
        requirements,
        required_keys,
        covered_keys,
        selected,
        trace_candidates,
        max_results,
        source_count,
        document_count,
        section_count,
        stop_reason,
    } = summary;
    let stop_reason = match stop_reason {
        Some(reason) => reason,
        None if selected.is_empty() => SearchStopReason::NoEvidence,
        None if max_results > 0 && selected.len() >= max_results => SearchStopReason::ResultsLimit,
        None if requirements_satisfied(
            requirements,
            required_keys,
            covered_keys,
            source_count,
            document_count,
            section_count,
        ) =>
        {
            SearchStopReason::EvidenceComplete
        }
        None => SearchStopReason::RequirementsUnmet,
    };
    let coverage = build_coverage(
        requirements,
        required_keys,
        covered_keys,
        source_count,
        document_count,
        section_count,
    );
    let mut covered_keys_vec = covered_keys.iter().cloned().collect::<Vec<_>>();
    covered_keys_vec.sort();
    let trace = SearchTraceDiversity {
        distinct_sources: source_count,
        distinct_documents: document_count,
        distinct_sections: section_count,
        required_claims: requirements.required_claims.clone(),
        required_subquestions: requirements.required_subquestions.clone(),
        covered_keys: covered_keys_vec,
        stop_reason: stop_reason.clone(),
        candidates: trace_candidates,
    };
    (stop_reason, coverage, trace)
}

fn required_keys(requirements: &EvidenceRequirements) -> Vec<String> {
    let mut keys = requirements.required_claims.clone();
    keys.extend(requirements.required_subquestions.iter().cloned());
    keys.sort();
    keys.dedup();
    keys
}

fn candidate_metrics(
    candidate: &EvidenceCandidate,
    covered_keys: &BTreeSet<String>,
    sources: &BTreeSet<String>,
    documents: &BTreeSet<String>,
    sections: &BTreeSet<String>,
) -> (String, String, String, usize, usize) {
    let source = source_identity(candidate);
    let document = format!("artifact:{}", candidate.artifact_version.value());
    let section = section_identity(candidate);
    let marginal_coverage = candidate
        .coverage_keys
        .iter()
        .filter(|key| !covered_keys.contains(*key))
        .collect::<BTreeSet<_>>()
        .len();
    let marginal_gain = marginal_coverage
        + usize::from(!sources.contains(&source))
        + usize::from(!documents.contains(&document))
        + usize::from(!sections.contains(&section));
    (source, document, section, marginal_coverage, marginal_gain)
}

fn requirements_satisfied(
    requirements: &EvidenceRequirements,
    required_keys: &[String],
    covered_keys: &BTreeSet<String>,
    sources: usize,
    documents: usize,
    sections: usize,
) -> bool {
    required_keys.iter().all(|key| covered_keys.contains(key))
        && sources
            >= requirements
                .minimum_sources
                .max(usize::from(requirements.minimum_corroboration))
        && documents >= requirements.minimum_documents
        && sections >= requirements.minimum_sections
}

fn build_coverage(
    requirements: &EvidenceRequirements,
    required_keys: &[String],
    covered_keys: &BTreeSet<String>,
    source_count: usize,
    document_count: usize,
    section_count: usize,
) -> EvidenceCoverage {
    let missing = required_keys
        .iter()
        .filter(|key| !covered_keys.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    let matched = required_keys.len().saturating_sub(missing.len());
    let percent_covered = if source_count == 0 && document_count == 0 && section_count == 0 {
        0
    } else if required_keys.is_empty() {
        100
    } else {
        ((matched * 100) / required_keys.len()) as u8
    };
    let mut candidate_coverage_keys = covered_keys.iter().cloned().collect::<Vec<_>>();
    candidate_coverage_keys.sort();
    EvidenceCoverage {
        percent_covered,
        gaps_identified: missing,
        required_claims: requirements.required_claims.clone(),
        required_subquestions: requirements.required_subquestions.clone(),
        distinct_sources: source_count,
        distinct_documents: document_count,
        distinct_sections: section_count,
        candidate_coverage_keys,
    }
}
fn selection_status(
    selected: &[RankedCandidate],
    coverage: &EvidenceCoverage,
    requirements: &EvidenceRequirements,
    stop_reason: &SearchStopReason,
) -> SearchStatus {
    if selected.is_empty() {
        return SearchStatus::NoEvidenceFound;
    }
    if selected
        .iter()
        .all(|candidate| matches!(candidate.candidate.freshness, FreshnessStatus::Stale))
    {
        return SearchStatus::StaleEvidenceOnly;
    }
    let diversity_satisfied = coverage.distinct_sources
        >= requirements
            .minimum_sources
            .max(usize::from(requirements.minimum_corroboration))
        && coverage.distinct_documents >= requirements.minimum_documents
        && coverage.distinct_sections >= requirements.minimum_sections;
    if coverage.percent_covered < 100 {
        SearchStatus::EvidenceIncomplete
    } else if !diversity_satisfied || matches!(stop_reason, SearchStopReason::RequirementsUnmet) {
        SearchStatus::AnswerableWithWarnings
    } else {
        SearchStatus::Answerable
    }
}

fn trace_candidate(
    candidate: &EvidenceCandidate,
    original_rank: usize,
    selected_rank: Option<usize>,
    marginal_coverage: usize,
) -> SearchTraceDiversityCandidate {
    SearchTraceDiversityCandidate {
        candidate_id: candidate.evidence_id,
        original_rank,
        selected_rank,
        duplicate_cluster: candidate.duplicate_cluster,
        marginal_coverage: marginal_coverage.min(u8::MAX as usize) as u8,
        coverage_keys: candidate.coverage_keys.clone(),
    }
}

fn source_identity(candidate: &EvidenceCandidate) -> String {
    match candidate.source_span.location() {
        SourceLocation::File { path, .. } | SourceLocation::Symbol { path, .. } => path.clone(),
        SourceLocation::Page { .. } | SourceLocation::Region { .. } => {
            format!("artifact:{}", candidate.artifact_version.value())
        }
    }
}

fn section_identity(candidate: &EvidenceCandidate) -> String {
    let artifact = candidate.artifact_version.value();
    if let Some(node_id) = candidate.source_span.node_id() {
        return format!("artifact:{artifact}:node:{}", node_id.value());
    }
    match candidate.source_span.location() {
        SourceLocation::File {
            path,
            start_line,
            end_line,
        } => format!("artifact:{artifact}:file:{path}:{start_line}:{end_line}"),
        SourceLocation::Symbol {
            path,
            qualified_name,
        } => format!("artifact:{artifact}:symbol:{path}:{qualified_name}"),
        SourceLocation::Page {
            page_start,
            page_end,
        } => format!("artifact:{artifact}:page:{page_start}:{page_end}"),
        SourceLocation::Region {
            page,
            x,
            y,
            width,
            height,
        } => format!("artifact:{artifact}:region:{page}:{x}:{y}:{width}:{height}"),
    }
}
