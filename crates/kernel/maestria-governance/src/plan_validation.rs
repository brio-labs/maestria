use std::collections::BTreeSet;
use std::fmt;

use maestria_domain::{
    CorpusScope, CorpusSnapshotId, FreshnessRequirement, IndexGenerationId, Modality, ScopeId,
    SearchCompatibilityError, SearchIntent, SearchPlan, SearchStage,
};

use crate::RetrievalSecurityPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchPlanValidationError {
    Schema(SearchCompatibilityError),
    IntentMismatch {
        declared: SearchIntent,
        classified: SearchIntent,
    },
    UnsupportedIntent(SearchIntent),
    UnsupportedStage(SearchStage),
    UnsupportedModality(Modality),
    SnapshotUnavailable(CorpusSnapshotId),
    GenerationUnavailable(IndexGenerationId),
    ScopeDenied,
    TooManyScopes {
        requested: usize,
        allowed: u32,
    },
    FreshnessUnsupported,
    BudgetExceeded {
        budget: &'static str,
        requested: u32,
        allowed: u32,
    },
    SecurityCapabilityMissing(&'static str),
    WebCapabilityMissing,
}

impl fmt::Display for SearchPlanValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "search plan schema rejected: {error}"),
            Self::IntentMismatch {
                declared,
                classified,
            } => write!(
                f,
                "declared search intent {declared:?} does not match deterministic classification {classified:?}"
            ),
            Self::UnsupportedIntent(intent) => write!(f, "unsupported search intent: {intent:?}"),
            Self::UnsupportedStage(stage) => write!(f, "unsupported search stage: {stage:?}"),
            Self::UnsupportedModality(modality) => {
                write!(f, "unsupported search modality: {modality:?}")
            }
            Self::SnapshotUnavailable(snapshot) => {
                write!(f, "corpus snapshot {} is unavailable", snapshot.value())
            }
            Self::GenerationUnavailable(generation) => {
                write!(f, "index generation {} is unavailable", generation.value())
            }
            Self::ScopeDenied => write!(f, "search plan scope is not allowed by policy"),
            Self::TooManyScopes { requested, allowed } => {
                write!(
                    f,
                    "search plan requests {requested} scopes; maximum is {allowed}"
                )
            }
            Self::FreshnessUnsupported => write!(f, "freshness requirement is unsupported"),
            Self::BudgetExceeded {
                budget,
                requested,
                allowed,
            } => write!(
                f,
                "{budget} budget requests {requested}; capability allows {allowed}"
            ),
            Self::SecurityCapabilityMissing(capability) => {
                write!(
                    f,
                    "required security capability is unavailable: {capability}"
                )
            }
            Self::WebCapabilityMissing => write!(f, "web retrieval capability is unavailable"),
        }
    }
}

impl std::error::Error for SearchPlanValidationError {}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchCapabilities {
    intents: BTreeSet<SearchIntent>,
    stages: BTreeSet<SearchStage>,
    modalities: BTreeSet<Modality>,
    snapshots: BTreeSet<CorpusSnapshotId>,
    generations: BTreeSet<IndexGenerationId>,
    allowed_scopes: Option<BTreeSet<ScopeId>>,
    global_scope: bool,
    max_scope_ids: u32,
    supports_realtime: bool,
    max_age_days: Option<u32>,
    web_enabled: bool,
    acl_filtering: bool,
    trust_filtering: bool,
    sensitivity_filtering: bool,
    quarantine_filtering: bool,
    max_tokens: u32,
    max_latency_ms: u32,
    max_queries: u32,
    max_stages: u32,
    max_web_requests: u32,
}

impl SearchCapabilities {
    pub fn new() -> Self {
        Self {
            max_scope_ids: u32::MAX,
            ..Self::default()
        }
    }

    pub fn core_defaults(
        snapshot: CorpusSnapshotId,
        generation: IndexGenerationId,
        fingerprint_limits: (u32, u32),
    ) -> Self {
        Self::new()
            .with_intent(SearchIntent::ExactLookup)
            .with_intent(SearchIntent::FactualLocal)
            .with_stage(SearchStage::InitialRetrieval)
            .with_modality(Modality::Text)
            .with_snapshot(snapshot)
            .with_generation(generation)
            .allow_global_scope()
            .max_scope_ids(1)
            .max_budgets(fingerprint_limits.0, fingerprint_limits.1, 1, 1, 0)
            .with_security_filters()
    }

    pub fn with_intent(mut self, intent: SearchIntent) -> Self {
        self.intents.insert(intent);
        self
    }

    pub fn with_stage(mut self, stage: SearchStage) -> Self {
        self.stages.insert(stage);
        self
    }

    pub fn with_modality(mut self, modality: Modality) -> Self {
        self.modalities.insert(modality);
        self
    }

    pub fn with_snapshot(mut self, snapshot: CorpusSnapshotId) -> Self {
        self.snapshots.insert(snapshot);
        self
    }

    pub fn with_generation(mut self, generation: IndexGenerationId) -> Self {
        self.generations.insert(generation);
        self
    }

    pub fn allow_global_scope(mut self) -> Self {
        self.global_scope = true;
        self
    }

    pub fn with_allowed_scopes(mut self, scopes: impl IntoIterator<Item = ScopeId>) -> Self {
        self.allowed_scopes = Some(scopes.into_iter().collect());
        self
    }

    pub fn max_scope_ids(mut self, max_scope_ids: u32) -> Self {
        self.max_scope_ids = max_scope_ids;
        self
    }

    pub fn support_realtime(mut self) -> Self {
        self.supports_realtime = true;
        self
    }

    pub fn support_max_age_days(mut self, max_age_days: u32) -> Self {
        self.max_age_days = Some(max_age_days);
        self
    }

    pub fn enable_web(mut self) -> Self {
        self.web_enabled = true;
        self
    }

    pub fn with_security_filters(mut self) -> Self {
        self.acl_filtering = true;
        self.trust_filtering = true;
        self.sensitivity_filtering = true;
        self.quarantine_filtering = true;
        self
    }

    pub fn max_budgets(
        mut self,
        max_tokens: u32,
        max_latency_ms: u32,
        max_queries: u32,
        max_stages: u32,
        max_web_requests: u32,
    ) -> Self {
        self.max_tokens = max_tokens;
        self.max_latency_ms = max_latency_ms;
        self.max_queries = max_queries;
        self.max_stages = max_stages;
        self.max_web_requests = max_web_requests;
        self
    }
}

pub struct SearchPlanValidator;

impl SearchPlanValidator {
    pub fn validate(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
        policy: &RetrievalSecurityPolicy,
    ) -> Result<(), SearchPlanValidationError> {
        plan.validate_schema()
            .map_err(SearchPlanValidationError::Schema)?;
        let classified = SearchIntent::classify(&plan.original_query);
        if classified != plan.intent {
            return Err(SearchPlanValidationError::IntentMismatch {
                declared: plan.intent,
                classified,
            });
        }
        if !capabilities.intents.contains(&plan.intent) {
            return Err(SearchPlanValidationError::UnsupportedIntent(plan.intent));
        }
        if let Some(stage) = plan
            .stages
            .iter()
            .find(|stage| !capabilities.stages.contains(stage))
        {
            return Err(SearchPlanValidationError::UnsupportedStage(*stage));
        }
        if let Some(modality) = plan
            .modalities
            .values()
            .iter()
            .find(|modality| !capabilities.modalities.contains(modality))
        {
            return Err(SearchPlanValidationError::UnsupportedModality(*modality));
        }
        if !capabilities.snapshots.contains(&plan.corpus_snapshot) {
            return Err(SearchPlanValidationError::SnapshotUnavailable(
                plan.corpus_snapshot,
            ));
        }
        if !capabilities.generations.contains(&plan.index_generation) {
            return Err(SearchPlanValidationError::GenerationUnavailable(
                plan.index_generation,
            ));
        }
        Self::validate_scope(plan, capabilities, policy)?;
        Self::validate_freshness(plan, capabilities)?;
        Self::validate_budgets(plan, capabilities)?;
        Self::validate_security(capabilities, policy)?;
        if (plan.intent == SearchIntent::CurrentWeb
            || plan.modalities.values().contains(&Modality::Web))
            && !capabilities.web_enabled
        {
            return Err(SearchPlanValidationError::WebCapabilityMissing);
        }
        Ok(())
    }

    fn validate_scope(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
        policy: &RetrievalSecurityPolicy,
    ) -> Result<(), SearchPlanValidationError> {
        match &plan.scope {
            CorpusScope::Global
                if !capabilities.global_scope
                    || capabilities
                        .allowed_scopes
                        .as_ref()
                        .is_some_and(|scopes| !scopes.is_empty()) =>
            {
                Err(SearchPlanValidationError::ScopeDenied)
            }
            CorpusScope::Global => Ok(()),
            CorpusScope::Restricted(scopes) => {
                if scopes.len() > capabilities.max_scope_ids as usize {
                    return Err(SearchPlanValidationError::TooManyScopes {
                        requested: scopes.len(),
                        allowed: capabilities.max_scope_ids,
                    });
                }
                if policy
                    .required_scope_id
                    .is_some_and(|required| scopes.len() != 1 || scopes.first() != Some(&required))
                {
                    return Err(SearchPlanValidationError::ScopeDenied);
                }
                if let Some(allowed) = &capabilities.allowed_scopes
                    && scopes.iter().any(|scope| !allowed.contains(scope))
                {
                    return Err(SearchPlanValidationError::ScopeDenied);
                }
                Ok(())
            }
        }
    }

    fn validate_freshness(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
    ) -> Result<(), SearchPlanValidationError> {
        match plan.freshness {
            FreshnessRequirement::Any => Ok(()),
            FreshnessRequirement::Realtime if capabilities.supports_realtime => Ok(()),
            FreshnessRequirement::MaximumAgeDays(days)
                if capabilities.max_age_days.is_some_and(|max| days <= max) =>
            {
                Ok(())
            }
            _ => Err(SearchPlanValidationError::FreshnessUnsupported),
        }
    }

    fn validate_budgets(
        plan: &SearchPlan,
        capabilities: &SearchCapabilities,
    ) -> Result<(), SearchPlanValidationError> {
        let budgets = [
            ("token", plan.budgets.max_tokens(), capabilities.max_tokens),
            (
                "latency_ms",
                plan.budgets.max_latency_ms(),
                capabilities.max_latency_ms,
            ),
            (
                "query",
                plan.budgets.max_queries(),
                capabilities.max_queries,
            ),
            ("stage", plan.budgets.max_stages(), capabilities.max_stages),
            (
                "web_request",
                plan.budgets.max_web_requests(),
                capabilities.max_web_requests,
            ),
        ];
        budgets
            .into_iter()
            .find_map(|(budget, requested, allowed)| {
                (requested > allowed).then_some(SearchPlanValidationError::BudgetExceeded {
                    budget,
                    requested,
                    allowed,
                })
            })
            .map_or(Ok(()), Err)
    }

    fn validate_security(
        capabilities: &SearchCapabilities,
        policy: &RetrievalSecurityPolicy,
    ) -> Result<(), SearchPlanValidationError> {
        if !capabilities.acl_filtering && policy.require_read_allowed {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing("ACL"));
        }
        if !capabilities.trust_filtering && policy.require_trust_zone.is_some() {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing(
                "trust filtering",
            ));
        }
        if !capabilities.sensitivity_filtering && policy.max_sensitivity.is_some() {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing(
                "sensitivity filtering",
            ));
        }
        if !capabilities.quarantine_filtering {
            return Err(SearchPlanValidationError::SecurityCapabilityMissing(
                "quarantine filtering",
            ));
        }
        Ok(())
    }
}
