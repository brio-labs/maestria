use std::collections::BTreeSet;

use maestria_domain::{EvidenceKind, FreshnessRequirement, FreshnessStatus, SearchStatus};

use super::types::{
    SearchValidationContext, Severity, ValidationCheck, ValidationContext, Validator,
};

fn passed_check(name: &str, message: impl Into<String>) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: true,
        severity: Severity::Error,
        message: message.into(),
    }
}

fn failed_check(name: &str, message: impl Into<String>) -> ValidationCheck {
    ValidationCheck {
        name: name.to_string(),
        passed: false,
        severity: Severity::Error,
        message: message.into(),
    }
}

pub(crate) fn evaluate_search(
    context: &ValidationContext<'_>,
    name: &str,
    check: impl FnOnce(&SearchValidationContext<'_>) -> Result<String, String>,
) -> ValidationCheck {
    let Some(search) = context.search.as_ref() else {
        return passed_check(name, "no search outcome is present");
    };
    match check(search) {
        Ok(message) => passed_check(name, message),
        Err(message) => failed_check(name, message),
    }
}

fn outcome_evidence_ids(
    search: &SearchValidationContext<'_>,
) -> BTreeSet<maestria_domain::EvidenceId> {
    search.candidate_ids().collect()
}

fn is_primary_source(evidence: &maestria_domain::Evidence) -> bool {
    match &evidence.kind {
        EvidenceKind::WebSnapshot { metadata, .. } => metadata.primary_source,
        _ => true,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SearchPlanValidator;

impl Validator for SearchPlanValidator {
    fn name(&self) -> &str {
        "search_plan"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
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
            if trace.budgets.max_tokens() == 0
                || trace.budgets.max_latency_ms() == 0
                || trace.budgets.max_queries() == 0
                || trace.budgets.max_stages() == 0
            {
                errors.push("search trace contains a zero execution budget".to_string());
            }
            if trace.stop_conditions.max_results == 0 {
                errors.push("search trace has no result limit".to_string());
            }
            if trace.deterministic_id() != search.outcome.trace {
                errors.push("search trace identity does not match the outcome".to_string());
            }
            if let Err(error) = trace.validate_rewrites() {
                errors.push(format!("rewrite provenance is invalid: {error}"));
            }
            let Some(plan) = search.plan else {
                return Err("search validation requires the persisted SearchPlan".to_string());
            };
            if let Err(error) = plan.validate_schema() {
                errors.push(format!("search plan schema is invalid: {error}"));
            }
            if !trace.matches_plan(plan) {
                errors.push("SearchTrace does not match the SearchPlan".to_string());
            }
            if errors.is_empty() {
                Ok("search plan and trace schema are valid".to_string())
            } else {
                Err(errors.join("; "))
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CoverageValidator;

impl Validator for CoverageValidator {
    fn name(&self) -> &str {
        "coverage"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
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
            if search.outcome.coverage.percent_covered == 0 {
                errors.push("coverage is zero for the search outcome".to_string());
            }
            if search.outcome.status == SearchStatus::Answerable
                && (search.outcome.coverage.percent_covered != 100
                    || !search.outcome.coverage.gaps_identified.is_empty())
            {
                errors.push("Answerable outcome has incomplete coverage".to_string());
            }
            if let Some(trace) = search.trace {
                let requirements = &trace.evidence_requirements;
                if search.outcome.coverage.required_claims != requirements.required_claims {
                    errors
                        .push("required claim coverage does not match the SearchTrace".to_string());
                }
                if search.outcome.coverage.required_subquestions
                    != requirements.required_subquestions
                {
                    errors.push(
                        "required subquestion coverage does not match the SearchTrace".to_string(),
                    );
                }
                if search.outcome.evidence.len() < usize::from(requirements.minimum_corroboration) {
                    errors.push("minimum corroboration is not satisfied".to_string());
                }
                if search.outcome.coverage.distinct_sources < requirements.minimum_sources {
                    errors.push("minimum source coverage is not satisfied".to_string());
                }
                if search.outcome.coverage.distinct_documents < requirements.minimum_documents {
                    errors.push("minimum document coverage is not satisfied".to_string());
                }
                if search.outcome.coverage.distinct_sections < requirements.minimum_sections {
                    errors.push("minimum section coverage is not satisfied".to_string());
                }
                if requirements.require_primary_sources
                    && !search.outcome.evidence.iter().any(|candidate| {
                        search
                            .evidence_record(candidate.evidence_id)
                            .is_some_and(is_primary_source)
                    })
                {
                    errors.push("required primary-source evidence is absent".to_string());
                }
                if !trace.matches_coverage(
                    &search.outcome.coverage,
                    &search.outcome.conflicts,
                    search.outcome.evidence.len(),
                ) {
                    errors.push("coverage does not match the SearchTrace".to_string());
                }
            }
            if errors.is_empty() {
                Ok(format!(
                    "coverage is {}% across {} candidate(s)",
                    search.outcome.coverage.percent_covered,
                    search.outcome.evidence.len()
                ))
            } else {
                Err(errors.join("; "))
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConflictValidator;

impl Validator for ConflictValidator {
    fn name(&self) -> &str {
        "conflict"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
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
                    .any(|candidate| !candidate_ids.contains(&candidate.evidence_id))
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
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FreshnessValidator;

impl Validator for FreshnessValidator {
    fn name(&self) -> &str {
        "freshness"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
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
                .filter(|candidate| candidate.freshness != FreshnessStatus::UpToDate)
                .count();
            if stale_count == 0 {
                Ok("all candidates satisfy the freshness requirement".to_string())
            } else {
                Err(format!(
                    "{stale_count} candidate(s) are stale or unknown under {:?}",
                    trace.freshness
                ))
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CitationAlignmentValidator;

impl Validator for CitationAlignmentValidator {
    fn name(&self) -> &str {
        "citation_alignment"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
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
        })
    }
}
