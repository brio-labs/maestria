use maestria_domain::{
    CorpusScope, FreshnessRequirement, ScopeId, SearchStatus, SearchStopReason, SearchTraceFilter,
    Sensitivity, TrustZone,
};

use super::search_validators::evaluate_search;
use super::types::{SearchValidationContext, ValidationCheck, ValidationContext, Validator};
fn denied_candidate_count(
    search: &SearchValidationContext<'_>,
    scope: &CorpusScope,
    required_trust: Option<&TrustZone>,
    maximum_sensitivity: Option<&Sensitivity>,
    required_scope: Option<ScopeId>,
    policy_allows_unscoped: bool,
) -> usize {
    let restricted_scopes = match scope {
        CorpusScope::Restricted(scopes) => Some(scopes),
        CorpusScope::Global => None,
    };
    search
        .outcome
        .evidence
        .iter()
        .filter(|candidate| {
            let Some(evidence) = search.evidence_record(candidate.evidence_id) else {
                return false;
            };
            let Some(artifact) = search.artifact_record(evidence.artifact_id) else {
                return true;
            };
            let security = artifact.security.taint_from(&evidence.security);
            let scope_denied = restricted_scopes.is_some_and(|scopes| {
                security
                    .scope_id
                    .is_none_or(|scope| !scopes.contains(&scope))
                    && !(policy_allows_unscoped && security.scope_id.is_none())
            }) || required_scope.is_some_and(|scope| {
                security.scope_id != Some(scope)
                    && !(policy_allows_unscoped && security.scope_id.is_none())
            });
            let trust_denied = required_trust.is_some_and(|trust| security.trust_zone != *trust);
            let sensitivity_denied = maximum_sensitivity.is_some_and(|maximum| {
                sensitivity_level(&security.sensitivity) > sensitivity_level(maximum)
            });
            scope_denied
                || trust_denied
                || sensitivity_denied
                || !security.retrieval_allowed()
                || security.prompt_injection_risk
                || !security.poisoning_flags.is_empty()
        })
        .count()
}
fn policy_trust(policy: &str) -> Option<TrustZone> {
    [
        ("System", TrustZone::System),
        ("Verified", TrustZone::Verified),
        ("Untrusted", TrustZone::Untrusted),
        ("Quarantined", TrustZone::Quarantined),
    ]
    .into_iter()
    .find_map(|(name, zone)| {
        policy
            .contains(&format!("trust=Some({name})"))
            .then_some(zone)
    })
}

fn policy_sensitivity(policy: &str) -> Option<Sensitivity> {
    [
        ("Public", Sensitivity::Public),
        ("Internal", Sensitivity::Internal),
        ("Confidential", Sensitivity::Confidential),
        ("Restricted", Sensitivity::Restricted),
    ]
    .into_iter()
    .find_map(|(name, sensitivity)| {
        policy
            .contains(&format!("sensitivity=Some({name})"))
            .then_some(sensitivity)
    })
}

fn policy_scope(policy: &str) -> Option<ScopeId> {
    policy.split(';').find_map(|field| {
        let value = field
            .strip_prefix("scope=Some(ScopeId(")?
            .strip_suffix("))")?;
        value.parse().ok().map(ScopeId::new)
    })
}

fn sensitivity_level(sensitivity: &Sensitivity) -> u8 {
    match sensitivity {
        Sensitivity::Public => 0,
        Sensitivity::Internal => 1,
        Sensitivity::Confidential => 2,
        Sensitivity::Restricted => 3,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RetrievalSecurityValidator;

impl Validator for RetrievalSecurityValidator {
    fn name(&self) -> &str {
        "retrieval_security"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
            let Some(trace) = search.trace else {
                return Err(
                    "retrieval security cannot be checked without a SearchTrace".to_string()
                );
            };
            let Some(policy) = trace
                .policy_fingerprint
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            else {
                return Err("retrieval security requires a policy fingerprint".to_string());
            };
            let required_trust = policy_trust(policy);
            let maximum_sensitivity = policy_sensitivity(policy);
            let required_scope = policy_scope(policy);
            let policy_allows_unscoped = policy.contains("unscoped=true");
            let malformed_policy = (policy.contains("trust=Some(") && required_trust.is_none())
                || (policy.contains("sensitivity=Some(") && maximum_sensitivity.is_none())
                || (policy.contains("scope=Some(") && required_scope.is_none());
            let mut required_filters = vec![
                SearchTraceFilter::Quarantine,
                SearchTraceFilter::PromptInjection,
            ];
            if matches!(trace.scope, CorpusScope::Restricted(_)) || policy.contains("scope=Some") {
                required_filters.push(SearchTraceFilter::Scope);
            }
            if policy.contains("read_allowed=true") {
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
                required_trust.as_ref(),
                maximum_sensitivity.as_ref(),
                required_scope,
                policy_allows_unscoped,
            );
            let missing_records = search
                .outcome
                .evidence
                .iter()
                .filter(|candidate| search.evidence_record(candidate.evidence_id).is_none())
                .count();
            let malformed_policy_count = usize::from(malformed_policy);
            if missing_filters == 0
                && denied_count == 0
                && missing_records == 0
                && malformed_policy_count == 0
            {
                Ok("retrieval filters and evidence security metadata permit release".to_string())
            } else {
                Err(format!(
                    "retrieval security failed: {missing_filters} required filter(s) missing, {denied_count} denied candidate(s), {missing_records} missing record(s), {malformed_policy_count} malformed policy fingerprint(s)"
                ))
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SearchRegressionValidator;

impl Validator for SearchRegressionValidator {
    fn name(&self) -> &str {
        "search_regression"
    }

    fn validate(&self, context: &ValidationContext<'_>) -> ValidationCheck {
        evaluate_search(context, self.name(), |search| {
            let Some(trace) = search.trace else {
                return Err("search regression checks require a SearchTrace".to_string());
            };
            let mut errors = Vec::new();
            if trace.deterministic_id() != search.outcome.trace {
                errors.push("trace identity changed without updating the outcome".to_string());
            }
            if trace.fingerprint != search.outcome.fingerprint {
                errors.push("retrieval model fingerprint differs from the trace".to_string());
            }
            if trace.index_generation != search.outcome.index_generation {
                errors.push("index generation differs from the trace".to_string());
            }
            if !trace.matches_evidence(&search.outcome.evidence) {
                errors.push("candidate order or provenance differs from the trace".to_string());
            }
            if !trace.matches_coverage(
                &search.outcome.coverage,
                &search.outcome.conflicts,
                search.outcome.evidence.len(),
            ) {
                errors.push("coverage differs from the trace".to_string());
            }
            if !trace.matches_outcome(&search.outcome.status, search.outcome.evidence.len()) {
                errors.push("stop reason is incompatible with the outcome".to_string());
            }
            if search.has_duplicate_candidates() {
                errors.push("outcome contains duplicate candidate ids".to_string());
            }
            if trace.stop_reason == SearchStopReason::EvidenceComplete
                && search.outcome.coverage.percent_covered != 100
                && search.outcome.status == SearchStatus::Answerable
            {
                errors.push(
                    "evidence-complete trace has an incomplete answerable outcome".to_string(),
                );
            }
            if errors.is_empty() {
                Ok("search trace and outcome are reproducible".to_string())
            } else {
                Err(errors.join("; "))
            }
        })
    }
}
