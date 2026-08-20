use maestria_domain::{CorpusScope, FreshnessRequirement, SearchStatus, SearchTraceFilter};

use super::rules::{
    candidate_matches_record, denied_candidate_count, is_primary_source, outcome_evidence_ids,
    policy_snapshot,
};
use crate::types::ValidationContext;

pub(crate) fn check_search_plan(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let Some(trace) = search.trace else {
        return Err("search outcome is missing its SearchTrace".to_string());
    };
    let mut errors = Vec::new();
    if trace.original_query.trim().is_empty() {
        errors.push("original query is empty".to_string());
    }
    if trace.stages.is_empty()
        || trace.stages.first() != Some(&maestria_domain::SearchStage::InitialRetrieval)
    {
        errors.push("search stages do not begin with initial retrieval".to_string());
    }
    if !search.trace_identity_matches() {
        errors.push("search trace identity does not match the outcome".to_string());
    }
    if let Err(error) = trace.validate_rewrites() {
        errors.push(format!("rewrite provenance is invalid: {error}"));
    }
    let Some(plan) = search.plan else {
        return Err("search validation requires the persisted SearchPlan".to_string());
    };
    if !trace.matches_plan(plan) {
        errors.push("SearchTrace does not match the SearchPlan".to_string());
    }
    if errors.is_empty() {
        Ok("search plan and trace schema are valid".to_string())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn check_coverage(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let mut errors = Vec::new();
    if search.outcome.evidence.is_empty() {
        errors.push("evidence is absent for the search outcome".to_string());
    }
    if !matches!(search.outcome.status, SearchStatus::Answerable) {
        errors.push(format!(
            "search status {:?} is not eligible for verified completion",
            search.outcome.status
        ));
    }
    if search.outcome.coverage.percent_covered() == 0 {
        errors.push("coverage is zero for the search outcome".to_string());
    }
    if search.outcome.status == SearchStatus::Answerable
        && (search.outcome.coverage.percent_covered() != 100
            || !search.outcome.coverage.gaps_identified().is_empty())
    {
        errors.push("Answerable outcome has incomplete coverage".to_string());
    }
    if let Some(trace) = search.trace {
        let requirements = &trace.evidence_requirements;
        if search.outcome.coverage.required_claims() != requirements.required_claims {
            errors.push("required claim coverage does not match the SearchTrace".to_string());
        }
        if search.outcome.coverage.required_subquestions() != requirements.required_subquestions {
            errors.push("required subquestion coverage does not match the SearchTrace".to_string());
        }
        if search.outcome.evidence.len() < usize::from(requirements.minimum_corroboration) {
            errors.push("minimum corroboration is not satisfied".to_string());
        }
        if search.outcome.coverage.distinct_sources() < requirements.minimum_sources {
            errors.push("minimum source coverage is not satisfied".to_string());
        }
        if search.outcome.coverage.distinct_documents() < requirements.minimum_documents {
            errors.push("minimum document coverage is not satisfied".to_string());
        }
        if search.outcome.coverage.distinct_sections() < requirements.minimum_sections {
            errors.push("minimum section coverage is not satisfied".to_string());
        }
        if requirements.require_primary_sources
            && !search.outcome.evidence.iter().any(|candidate| {
                search
                    .evidence_record(candidate.evidence_id())
                    .is_some_and(is_primary_source)
            })
        {
            errors.push("required primary-source evidence is absent".to_string());
        }
        if !search.coverage_matches_trace() {
            errors.push("coverage does not match the SearchTrace".to_string());
        }
    }
    if errors.is_empty() {
        Ok(format!(
            "coverage is {}% across {} candidate(s)",
            search.outcome.coverage.percent_covered(),
            search.outcome.evidence.len()
        ))
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn check_conflict(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let has_conflicts = !search.outcome.conflicts.is_empty();
    let status_is_conflict = search.outcome.status == SearchStatus::SourcesConflict;
    let mut errors = Vec::new();
    if has_conflicts != status_is_conflict {
        errors.push("conflict records and SourcesConflict status disagree".to_string());
    }
    let candidate_ids = outcome_evidence_ids(search);
    for conflict in &search.outcome.conflicts {
        if conflict.candidates.is_empty() {
            errors.push(format!("conflict {} has no candidates", conflict.id));
        }
        if conflict
            .candidates
            .iter()
            .any(|candidate| !candidate_ids.contains(&candidate.evidence_id()))
        {
            errors.push(format!(
                "conflict {} references a candidate outside the outcome",
                conflict.id
            ));
        }
    }
    if let Some(trace) = search.trace {
        let conflict_ids = search
            .outcome
            .conflicts
            .iter()
            .map(|conflict| conflict.id)
            .collect::<Vec<_>>();
        if trace.conflicts != conflict_ids {
            errors.push("conflict trace does not match the outcome".to_string());
        }
    }
    if errors.is_empty() {
        Ok("conflict status and source sets are consistent".to_string())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn check_freshness(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let Some(trace) = search.trace else {
        return Err("freshness cannot be checked without a SearchTrace".to_string());
    };
    if matches!(trace.freshness, FreshnessRequirement::Any) {
        return Ok("search accepts evidence of any age".to_string());
    }
    let stale_count = search
        .outcome
        .evidence
        .iter()
        .filter(|candidate| candidate.freshness() != maestria_domain::FreshnessStatus::UpToDate)
        .count();
    if stale_count == 0 {
        Ok("all candidates satisfy the freshness requirement".to_string())
    } else {
        Err(format!(
            "{stale_count} candidate(s) are stale or unknown under {:?}",
            trace.freshness
        ))
    }
}

pub(crate) fn check_citation_alignment(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let candidate_ids = outcome_evidence_ids(search);
    let misaligned_claims = context
        .claims
        .values()
        .filter(|claim| {
            claim.evidence_ids.is_empty()
                || !claim
                    .evidence_ids
                    .iter()
                    .any(|evidence_id| candidate_ids.contains(evidence_id))
        })
        .count();
    if misaligned_claims == 0 {
        Ok("claims are aligned with search candidates".to_string())
    } else {
        Err(format!(
            "{misaligned_claims} claim(s) are not aligned with search candidates"
        ))
    }
}

pub(crate) fn check_retrieval_security(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let Some(trace) = search.trace else {
        return Err("retrieval security cannot be checked without a SearchTrace".to_string());
    };
    let Some(policy_fingerprint) = trace
        .policy_fingerprint
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return Err("retrieval security requires a policy fingerprint".to_string());
    };
    let policy = policy_snapshot(policy_fingerprint)?;
    let required_trust = policy.require_trust_zone().cloned();
    let maximum_sensitivity = policy.max_sensitivity().cloned();
    let effective_scopes = policy.effective_scopes();
    let policy_allows_unscoped = policy.allows_unscoped_items();
    let mut required_filters = vec![
        SearchTraceFilter::Quarantine,
        SearchTraceFilter::PromptInjection,
    ];
    if matches!(trace.scope, CorpusScope::Restricted(_)) || effective_scopes.is_some() {
        required_filters.push(SearchTraceFilter::Scope);
    }
    if policy.requires_read_allowed() {
        required_filters.push(SearchTraceFilter::Acl);
    }
    if required_trust.is_some() {
        required_filters.push(SearchTraceFilter::Trust);
    }
    if maximum_sensitivity.is_some() {
        required_filters.push(SearchTraceFilter::Sensitivity);
    }
    if !matches!(trace.freshness, FreshnessRequirement::Any) {
        required_filters.push(SearchTraceFilter::Freshness);
    }
    let missing_filters = required_filters
        .iter()
        .filter(|filter| !trace.filters.contains(filter))
        .count();
    let denied_count = denied_candidate_count(
        search,
        &trace.scope,
        effective_scopes,
        required_trust.as_ref(),
        maximum_sensitivity.as_ref(),
        policy_allows_unscoped,
    );
    let missing_records = search.missing_evidence_records();
    if missing_filters == 0 && denied_count == 0 && missing_records == 0 {
        Ok("retrieval filters and evidence security metadata permit release".to_string())
    } else {
        Err(format!(
            "retrieval security failed: {missing_filters} required filter(s) missing, \
             {denied_count} denied candidate(s), {missing_records} missing record(s)"
        ))
    }
}

pub(crate) fn check_search_regression(context: &ValidationContext<'_>) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let Some(trace) = search.trace else {
        return Err("search regression checks require a SearchTrace".to_string());
    };
    let mut errors = Vec::new();
    if !search.trace_identity_matches() {
        errors.push("trace identity changed without updating the outcome".to_string());
    }
    if !search.fingerprint_matches() {
        errors.push("retrieval model fingerprint differs from the trace".to_string());
    }
    if !search.index_generation_matches() {
        errors.push("index generation differs from the trace".to_string());
    }
    if !search.evidence_matches_trace() {
        errors.push("candidate order or provenance differs from the trace".to_string());
    }
    if !search.coverage_matches_trace() {
        errors.push("coverage differs from the trace".to_string());
    }
    if !search.outcome_matches_trace() {
        errors.push("stop reason is incompatible with the outcome".to_string());
    }
    if search.has_duplicate_candidates() {
        errors.push("outcome contains duplicate candidate ids".to_string());
    }
    if trace.stop_reason == maestria_domain::SearchStopReason::EvidenceComplete
        && search.outcome.coverage.percent_covered() != 100
        && search.outcome.status == SearchStatus::Answerable
    {
        errors.push("evidence-complete trace has an incomplete answerable outcome".to_string());
    }
    if errors.is_empty() {
        Ok("search trace and outcome are reproducible".to_string())
    } else {
        Err(errors.join("; "))
    }
}

pub(crate) fn check_candidate_provenance(
    context: &ValidationContext<'_>,
) -> Result<String, String> {
    let Some(search) = context.search.as_ref() else {
        return Ok("no search outcome is present".to_string());
    };
    let mut errors = Vec::new();
    for candidate in &search.outcome.evidence {
        let Some(evidence) = search.evidence_record(candidate.evidence_id()) else {
            errors.push(format!(
                "candidate {} has no evidence record",
                candidate.evidence_id().value()
            ));
            continue;
        };
        if !candidate_matches_record(candidate, evidence) {
            errors.push(format!(
                "candidate {} provenance does not match its evidence record",
                candidate.evidence_id().value()
            ));
        }
    }
    if errors.is_empty() {
        Ok("candidate provenance matches evidence records".to_string())
    } else {
        Err(errors.join("; "))
    }
}
