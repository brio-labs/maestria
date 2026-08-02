use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RewriteOrigin {
    Original,
    Deterministic,
    ModelProposal,
    Feedback,
    /// A rewrite that fills a declared missing-evidence slot; the slot
    /// identity lives on the variant so a missing-slot rewrite without a
    /// named slot is unrepresentable (R56).
    MissingSlot {
        slot: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StageRole {
    InitialRetrieval,
    Reranking,
    IterativeRetrieval,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RewriteAccounting {
    pub token_estimate: usize,
    pub latency_budget_units: u32,
    pub is_proposal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QueryRewriteRecord {
    pub query: String,
    pub origin: RewriteOrigin,
    pub stage: StageRole,
    pub accounting: RewriteAccounting,
}

#[derive(Debug, Clone)]
pub struct QueryRewriteSession {
    original_query: String,
    records: Vec<QueryRewriteRecord>,
    token_budget: usize,
    latency_budget_units: u32,
    query_budget: u32,
    used_tokens: usize,
    used_latency_budget_units: u32,
    deterministic_expanded: bool,
    allowed_missing_slots: BTreeSet<String>,
}

impl QueryRewriteSession {
    pub fn new(original_query: impl Into<String>) -> Self {
        Self::with_budget(original_query, usize::MAX, u32::MAX)
    }

    pub fn with_budget(
        original_query: impl Into<String>,
        token_budget: usize,
        latency_budget_units: u32,
    ) -> Self {
        Self::with_limits(original_query, token_budget, latency_budget_units, u32::MAX)
    }

    pub fn with_limits(
        original_query: impl Into<String>,
        token_budget: usize,
        latency_budget_units: u32,
        query_budget: u32,
    ) -> Self {
        let original_query = original_query.into();
        let original_record = QueryRewriteRecord {
            query: original_query.clone(),
            origin: RewriteOrigin::Original,
            stage: StageRole::InitialRetrieval,
            accounting: RewriteAccounting {
                token_estimate: Self::estimate_tokens(&original_query),
                latency_budget_units: 1,
                is_proposal: false,
            },
        };
        let used_tokens = original_record.accounting.token_estimate;
        let used_latency_budget_units = original_record.accounting.latency_budget_units;
        Self {
            original_query,
            records: vec![original_record],
            token_budget,
            latency_budget_units,
            query_budget,
            used_tokens,
            used_latency_budget_units,
            deterministic_expanded: false,
            allowed_missing_slots: BTreeSet::new(),
        }
    }

    pub fn original_query(&self) -> &str {
        &self.original_query
    }

    pub fn records(&self) -> &[QueryRewriteRecord] {
        &self.records
    }
    pub fn with_missing_slots(mut self, slots: impl IntoIterator<Item = String>) -> Self {
        self.allowed_missing_slots = slots.into_iter().collect();
        self
    }

    pub fn set_missing_slots(&mut self, slots: impl IntoIterator<Item = String>) {
        self.allowed_missing_slots = slots.into_iter().collect();
    }

    pub fn add_rewrite(&mut self, record: QueryRewriteRecord) -> bool {
        if record.origin == RewriteOrigin::Original
            && (record.query != self.original_query || record.stage != StageRole::InitialRetrieval)
        {
            return false;
        }
        if record.origin == RewriteOrigin::ModelProposal && !self.deterministic_expanded {
            return false;
        }
        if record.accounting.token_estimate != Self::estimate_tokens(&record.query)
            || record.accounting.latency_budget_units == 0
        {
            return false;
        }
        if !Self::policy_accepts(&record) {
            return false;
        }
        if let RewriteOrigin::MissingSlot { slot } = &record.origin
            && !self.allowed_missing_slots.contains(slot)
        {
            return false;
        }
        if self.used_tokens
            > self
                .token_budget
                .saturating_sub(record.accounting.token_estimate)
            || self.used_latency_budget_units
                > self
                    .latency_budget_units
                    .saturating_sub(record.accounting.latency_budget_units)
        {
            return false;
        }
        if self.records.iter().any(|existing| {
            existing.query == record.query
                && existing.origin == record.origin
                && existing.stage == record.stage
        }) {
            return false;
        }
        if self.records.len() >= self.query_budget as usize {
            return false;
        }
        self.used_tokens = self
            .used_tokens
            .saturating_add(record.accounting.token_estimate);
        self.used_latency_budget_units = self
            .used_latency_budget_units
            .saturating_add(record.accounting.latency_budget_units);
        self.records.push(record);
        self.sort_records();
        true
    }

    fn sort_records(&mut self) {
        self.records.sort_by(|a, b| {
            let rank = |o: &RewriteOrigin| match o {
                RewriteOrigin::Original => 0,
                RewriteOrigin::Deterministic => 1,
                RewriteOrigin::MissingSlot { .. } => 2,
                RewriteOrigin::Feedback => 3,
                RewriteOrigin::ModelProposal => 4,
            };
            rank(&a.origin)
                .cmp(&rank(&b.origin))
                .then_with(|| a.query.cmp(&b.query))
                .then_with(|| (a.stage as u8).cmp(&(b.stage as u8)))
        });
    }

    pub fn expand_deterministic(&mut self) {
        let mut new_records = Vec::new();
        for record in &self.records {
            if record.origin == RewriteOrigin::Original {
                let expansions = DeterministicRewriter::expand(&record.query);
                for expansion in expansions {
                    let token_estimate = Self::estimate_tokens(&expansion);
                    new_records.push(QueryRewriteRecord {
                        query: expansion,
                        origin: RewriteOrigin::Deterministic,
                        stage: StageRole::InitialRetrieval,
                        accounting: RewriteAccounting {
                            token_estimate,
                            latency_budget_units: 1,
                            is_proposal: false,
                        },
                    });
                }
            }
        }
        for r in new_records {
            self.add_rewrite(r);
        }
        self.deterministic_expanded = true;
    }

    pub fn add_missing_slot_rewrite(
        &mut self,
        query: impl Into<String>,
        slot: impl Into<String>,
        accounting: RewriteAccounting,
    ) -> bool {
        self.add_rewrite(QueryRewriteRecord {
            query: query.into(),
            origin: RewriteOrigin::MissingSlot { slot: slot.into() },
            stage: StageRole::IterativeRetrieval,
            accounting,
        })
    }

    fn estimate_tokens(s: &str) -> usize {
        s.split_whitespace().count().max(1)
    }

    pub fn trace_records(&self) -> Vec<maestria_domain::SearchTraceRewrite> {
        self.records
            .iter()
            .map(|record| maestria_domain::SearchTraceRewrite {
                query: record.query.clone(),
                origin: match &record.origin {
                    RewriteOrigin::Original => maestria_domain::SearchRewriteOrigin::Original,
                    RewriteOrigin::Deterministic => {
                        maestria_domain::SearchRewriteOrigin::Deterministic
                    }
                    RewriteOrigin::ModelProposal => {
                        maestria_domain::SearchRewriteOrigin::ModelProposal
                    }
                    RewriteOrigin::Feedback => maestria_domain::SearchRewriteOrigin::Feedback,
                    RewriteOrigin::MissingSlot { slot } => {
                        maestria_domain::SearchRewriteOrigin::MissingSlot { slot: slot.clone() }
                    }
                },
                stage: match record.stage {
                    StageRole::InitialRetrieval => {
                        maestria_domain::SearchRewriteStage::InitialRetrieval
                    }
                    StageRole::Reranking => maestria_domain::SearchRewriteStage::Reranking,
                    StageRole::IterativeRetrieval => {
                        maestria_domain::SearchRewriteStage::IterativeRetrieval
                    }
                },
                accounting: maestria_domain::SearchRewriteAccounting {
                    token_estimate: record.accounting.token_estimate.min(u32::MAX as usize) as u32,
                    latency_budget_units: record.accounting.latency_budget_units,
                    is_proposal: record.accounting.is_proposal,
                },
            })
            .collect()
    }

    pub fn policy_accepts(record: &QueryRewriteRecord) -> bool {
        let role_allowed = match &record.origin {
            RewriteOrigin::Original | RewriteOrigin::Deterministic => {
                record.stage == StageRole::InitialRetrieval
            }
            RewriteOrigin::MissingSlot { slot } => {
                record.stage == StageRole::IterativeRetrieval && !slot.trim().is_empty()
            }
            RewriteOrigin::ModelProposal | RewriteOrigin::Feedback => {
                matches!(
                    record.stage,
                    StageRole::Reranking | StageRole::IterativeRetrieval
                )
            }
        };
        if !role_allowed
            || record.accounting.is_proposal
                != matches!(record.origin, RewriteOrigin::ModelProposal)
        {
            return false;
        }
        true
    }
}

pub struct DeterministicRewriter;

impl DeterministicRewriter {
    pub fn expand(query: &str) -> Vec<String> {
        let mut results = Vec::new();

        // Aliases/acronyms
        if query.contains("PR") {
            results.push(query.replace("PR", "Pull Request"));
        }
        if query.contains("DB") {
            results.push(query.replace("DB", "Database"));
        }

        // Identifier normalization
        if query.contains("-") {
            results.push(query.replace("-", "_"));
        }

        // Project/path hints
        if query.contains("src/") {
            results.push(query.replace("src/", "crates/"));
        }
        if query.contains("test") && !query.contains("tests/") {
            results.push(query.replace("test", "tests/"));
        }

        // Code symbol variants
        if query.contains("fn ") {
            results.push(query.replace("fn ", "function "));
        }
        if query.contains("impl ") {
            results.push(query.replace("impl ", "implementation "));
        }
        if query.contains("module") {
            results.push(query.replace("module", "mod"));
        }

        // Entity IDs (e.g. #123 to Issue 123)
        if let Some(pos) = query.find('#') {
            let has_numeric_id = query
                .get(pos + 1..)
                .and_then(|suffix| suffix.chars().next())
                .is_some_and(|character| character.is_ascii_digit());
            if has_numeric_id {
                results.push(query.replace('#', "Issue "));
            }
        }

        // Date/version normalization
        if query.contains("v1.") {
            results.push(query.replace("v1.", "version 1."));
        }
        if query.contains("v2.") {
            results.push(query.replace("v2.", "version 2."));
        }

        for token in query.split_whitespace() {
            if Self::is_iso_date(token) {
                results.push(query.replace(token, &format!("date {token}")));
            }
        }

        results
    }

    fn is_iso_date(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
    }
}
