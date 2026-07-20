use std::collections::BTreeSet;

use super::{VisualBenchmarkCorpus, VisualBenchmarkError, VisualEvidenceKind, VisualQueryClass};

impl VisualBenchmarkCorpus {
    pub fn from_json(input: &str) -> Result<Self, VisualBenchmarkError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| VisualBenchmarkError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), VisualBenchmarkError> {
        self.validate_identity()?;
        self.validate_sources()?;
        let mut ids = BTreeSet::new();
        let mut classes = BTreeSet::new();
        let mut has_page = false;
        let mut has_region = false;
        for case in &self.cases {
            self.validate_case(case, &mut ids, &mut classes)?;
            for judgment in &case.judgments {
                self.validate_judgment(case.case_id.as_str(), judgment)?;
                has_page |= judgment.kind == VisualEvidenceKind::Page;
                has_region |= judgment.kind == VisualEvidenceKind::Region;
            }
        }
        for class in VisualQueryClass::all() {
            if !classes.contains(&class) {
                return Err(VisualBenchmarkError::MissingClass(class));
            }
        }
        if !has_page || !has_region {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "visual benchmark must contain page and region judgments".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_identity(&self) -> Result<(), VisualBenchmarkError> {
        if self.schema_version == 0
            || self.corpus_id.trim().is_empty()
            || self.corpus_revision.trim().is_empty()
        {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "schema and corpus identity must be non-empty".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_sources(&self) -> Result<(), VisualBenchmarkError> {
        if self.source_paths.is_empty()
            || self.source_paths.iter().any(|path| path.trim().is_empty())
        {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "visual benchmark source_paths must be non-empty".to_string(),
            ));
        }
        let source_paths = self.source_paths.iter().collect::<BTreeSet<_>>();
        if source_paths.len() != self.source_paths.len() {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "visual benchmark source_paths must be unique".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_case(
        &self,
        case: &super::VisualBenchmarkCase,
        ids: &mut BTreeSet<String>,
        classes: &mut BTreeSet<VisualQueryClass>,
    ) -> Result<(), VisualBenchmarkError> {
        if case.case_id.trim().is_empty() || case.query.trim().is_empty() {
            return Err(VisualBenchmarkError::InvalidCorpus(
                "case_id and query must be non-empty".to_string(),
            ));
        }
        if !ids.insert(case.case_id.clone()) {
            return Err(VisualBenchmarkError::DuplicateCase(case.case_id.clone()));
        }
        if case.judgments.is_empty()
            || case.latency_budget_ms == 0
            || case.memory_budget_bytes == 0
            || case.disk_budget_bytes == 0
            || case.energy_budget_millijoules == 0
        {
            return Err(VisualBenchmarkError::InvalidCorpus(format!(
                "case {} must have judgments and positive budgets",
                case.case_id
            )));
        }
        if VisualQueryClass::classify(&case.query) != Some(case.class) {
            return Err(VisualBenchmarkError::InvalidCorpus(format!(
                "case {} query does not classify as {:?}",
                case.case_id, case.class
            )));
        }
        if maestria_domain::SearchIntent::classify(&case.query)
            != maestria_domain::SearchIntent::VisualDocument
        {
            return Err(VisualBenchmarkError::InvalidCorpus(format!(
                "case {} is not a visual-document query",
                case.case_id
            )));
        }
        classes.insert(case.class);
        Ok(())
    }

    fn validate_judgment(
        &self,
        case_id: &str,
        judgment: &super::VisualJudgment,
    ) -> Result<(), VisualBenchmarkError> {
        if judgment.relevance == 0 {
            return Err(VisualBenchmarkError::InvalidCorpus(format!(
                "case {case_id} contains a zero-relevance judgment"
            )));
        }
        let evidence = &judgment.evidence;
        if !self.source_paths.contains(&evidence.source_path)
            || evidence.page == 0
            || evidence.width == 0
            || evidence.height == 0
        {
            return Err(VisualBenchmarkError::InvalidCorpus(format!(
                "case {case_id} contains an invalid source-backed evidence location"
            )));
        }
        Ok(())
    }

    pub(super) fn case(&self, case_id: &str) -> Option<&super::VisualBenchmarkCase> {
        self.cases.iter().find(|case| case.case_id == case_id)
    }
}
