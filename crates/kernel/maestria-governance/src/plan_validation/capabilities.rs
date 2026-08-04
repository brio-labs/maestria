use std::collections::BTreeSet;

use maestria_domain::{
    CorpusSnapshotId, IndexGenerationId, Modality, ScopeId, SearchIntent, SearchStage,
};

/// How a search capability scopes corpus access.
///
/// `Global` admits global-scope plans; `Restricted` admits exactly the listed
/// scopes; `Unscoped` (the default) denies global plans and applies no scope
/// filter to restricted plans. The former `global_scope` boolean plus
/// `allowed_scopes` option admitted the contradictory
/// `global_scope: true` + `allowed_scopes: Some(..)` combination and has been
/// replaced (R56).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ScopeMode {
    Global,
    Restricted(BTreeSet<ScopeId>),
    #[default]
    Unscoped,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchCapabilities {
    intents: BTreeSet<SearchIntent>,
    stages: BTreeSet<SearchStage>,
    modalities: BTreeSet<Modality>,
    snapshots: BTreeSet<CorpusSnapshotId>,
    generations: BTreeSet<IndexGenerationId>,
    scope: ScopeMode,
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
    max_bytes_read: u64,
    max_concurrency: u32,
}

impl SearchCapabilities {
    pub fn new() -> Self {
        Self {
            max_scope_ids: u32::MAX,
            max_bytes_read: u64::MAX,
            max_concurrency: u32::MAX,
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
        self.scope = ScopeMode::Global;
        self
    }

    pub fn with_allowed_scopes(mut self, scopes: impl IntoIterator<Item = ScopeId>) -> Self {
        self.scope = ScopeMode::Restricted(scopes.into_iter().collect());
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

    pub fn max_bytes_read(mut self, max_bytes_read: u64) -> Self {
        self.max_bytes_read = max_bytes_read;
        self
    }

    pub fn max_concurrency(mut self, max_concurrency: u32) -> Self {
        self.max_concurrency = max_concurrency;
        self
    }

    pub(crate) fn supports_intent(&self, intent: SearchIntent) -> bool {
        self.intents.contains(&intent)
    }

    pub(crate) fn supports_stage(&self, stage: &SearchStage) -> bool {
        self.stages.contains(stage)
    }

    pub(crate) fn supports_modality(&self, modality: &Modality) -> bool {
        self.modalities.contains(modality)
    }

    pub(crate) fn supports_snapshot(&self, snapshot: CorpusSnapshotId) -> bool {
        self.snapshots.contains(&snapshot)
    }

    pub(crate) fn supports_generation(&self, generation: IndexGenerationId) -> bool {
        self.generations.contains(&generation)
    }

    pub(crate) fn scope(&self) -> &ScopeMode {
        &self.scope
    }

    pub(crate) const fn scope_id_limit(&self) -> u32 {
        self.max_scope_ids
    }

    pub(crate) const fn supports_realtime(&self) -> bool {
        self.supports_realtime
    }

    pub(crate) const fn max_age_days(&self) -> Option<u32> {
        self.max_age_days
    }

    pub(crate) const fn web_enabled(&self) -> bool {
        self.web_enabled
    }

    pub(crate) const fn acl_filtering(&self) -> bool {
        self.acl_filtering
    }

    pub(crate) const fn trust_filtering(&self) -> bool {
        self.trust_filtering
    }

    pub(crate) const fn sensitivity_filtering(&self) -> bool {
        self.sensitivity_filtering
    }

    pub(crate) const fn quarantine_filtering(&self) -> bool {
        self.quarantine_filtering
    }

    pub(crate) const fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    pub(crate) const fn max_latency_ms(&self) -> u32 {
        self.max_latency_ms
    }

    pub(crate) const fn max_queries(&self) -> u32 {
        self.max_queries
    }

    pub(crate) const fn max_stages(&self) -> u32 {
        self.max_stages
    }

    pub(crate) const fn max_web_requests(&self) -> u32 {
        self.max_web_requests
    }

    pub(crate) const fn byte_limit(&self) -> u64 {
        self.max_bytes_read
    }

    pub(crate) const fn concurrency_limit(&self) -> u32 {
        self.max_concurrency
    }
}
