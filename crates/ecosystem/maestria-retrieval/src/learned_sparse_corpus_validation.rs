use std::collections::{BTreeMap, BTreeSet};

use super::{
    LEARNED_SPARSE_TASK_CORPUS_SCHEMA_VERSION, LearnedSparseCaseTag, LearnedSparseDataFidelity,
    LearnedSparseDataSplit, LearnedSparseJudgmentGuidance, LearnedSparseQueryClass,
    LearnedSparseQueryLanguage, LearnedSparseSourceInput, LearnedSparseTaskCase,
    LearnedSparseTaskCorpus, LearnedSparseTaskExpectation,
};

impl LearnedSparseTaskCorpus {
    pub fn from_json(input: &str) -> Result<Self, LearnedSparseCorpusError> {
        let corpus: Self = serde_json::from_str(input)
            .map_err(|error| LearnedSparseCorpusError::InvalidJson(error.to_string()))?;
        corpus.validate()?;
        Ok(corpus)
    }

    pub fn validate(&self) -> Result<(), LearnedSparseCorpusError> {
        if self.schema_version != LEARNED_SPARSE_TASK_CORPUS_SCHEMA_VERSION {
            return Err(LearnedSparseCorpusError::InvalidCorpus(
                "unsupported learned-sparse task corpus schema version".to_string(),
            ));
        }
        for (name, value) in [
            ("corpus_id", self.corpus_id.as_str()),
            ("corpus_revision", self.corpus_revision.as_str()),
            ("judgment_set_id", self.judgment_set_id.as_str()),
            ("evaluation_date", self.evaluation_date.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(LearnedSparseCorpusError::InvalidCorpus(format!(
                    "{name} must be non-empty"
                )));
            }
        }
        validate_sources(&self.source_inputs)?;
        validate_guidance(&self.judgment_guidance)?;
        validate_cases(&self.cases, &self.source_inputs)?;
        Ok(())
    }
}

fn validate_sources(sources: &[LearnedSparseSourceInput]) -> Result<(), LearnedSparseCorpusError> {
    if sources.is_empty() {
        return Err(LearnedSparseCorpusError::InvalidCorpus(
            "source input manifest must not be empty".to_string(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut has_task_definition = false;
    let mut has_evidence_source = false;
    let mut has_security_fixture = false;
    let mut has_judgment_guidance = false;
    for source in sources {
        if source.source_id.trim().is_empty()
            || !ids.insert(source.source_id.clone())
            || source.path.trim().is_empty()
            || source.path.starts_with('/')
            || source.path.split('/').any(|segment| segment == "..")
        {
            return Err(LearnedSparseCorpusError::InvalidSource(
                "source inputs require unique relative paths and identities".to_string(),
            ));
        }
        match source.role {
            super::LearnedSparseSourceRole::TaskDefinition => has_task_definition = true,
            super::LearnedSparseSourceRole::EvidenceSource => has_evidence_source = true,
            super::LearnedSparseSourceRole::SecurityFixture => has_security_fixture = true,
            super::LearnedSparseSourceRole::JudgmentGuidance => has_judgment_guidance = true,
        }
    }
    if !has_task_definition
        || !has_evidence_source
        || !has_security_fixture
        || !has_judgment_guidance
    {
        return Err(LearnedSparseCorpusError::InvalidSource(
            "source manifest must cover task, evidence, security, and guidance inputs".to_string(),
        ));
    }
    Ok(())
}

fn validate_guidance(
    guidance: &LearnedSparseJudgmentGuidance,
) -> Result<(), LearnedSparseCorpusError> {
    if guidance.independent_judges < 2 {
        return Err(LearnedSparseCorpusError::InvalidGuidance(
            "at least two independent judges are required".to_string(),
        ));
    }
    Ok(())
}

fn validate_cases(
    cases: &[LearnedSparseTaskCase],
    sources: &[LearnedSparseSourceInput],
) -> Result<(), LearnedSparseCorpusError> {
    if cases.is_empty() {
        return Err(LearnedSparseCorpusError::InvalidCorpus(
            "task corpus must contain cases".to_string(),
        ));
    }
    let source_ids = sources
        .iter()
        .map(|source| source.source_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut case_ids = BTreeSet::new();
    let mut final_counts = LearnedSparseQueryClass::all()
        .into_iter()
        .map(|class| (class, 0_u32))
        .collect::<BTreeMap<_, _>>();
    let mut final_task_ids = BTreeMap::<LearnedSparseQueryClass, BTreeSet<String>>::new();
    let mut has_development = false;
    let mut has_final = false;
    for case in cases {
        validate_case(case, &source_ids)?;
        if !case_ids.insert(case.case_id.clone()) {
            return Err(LearnedSparseCorpusError::DuplicateCase(
                case.case_id.clone(),
            ));
        }
        match case.split {
            LearnedSparseDataSplit::Development => has_development = true,
            LearnedSparseDataSplit::FinalEvaluation => {
                has_final = true;
                *final_counts.entry(case.class).or_insert(0_u32) += 1;
                final_task_ids
                    .entry(case.class)
                    .or_default()
                    .insert(case.task_id.clone());
            }
        }
    }
    if !has_development || !has_final {
        return Err(LearnedSparseCorpusError::InvalidCorpus(
            "development and final evaluation splits are both required".to_string(),
        ));
    }
    for class in LearnedSparseQueryClass::all() {
        let final_count = final_counts[&class];
        let distinct_tasks = final_task_ids.get(&class).map_or(0, BTreeSet::len);
        if final_count < 2 || distinct_tasks < 2 {
            return Err(LearnedSparseCorpusError::InvalidCorpus(format!(
                "final evaluation class {class:?} needs two independent task cases"
            )));
        }
    }
    Ok(())
}

fn validate_case(
    case: &LearnedSparseTaskCase,
    source_ids: &BTreeSet<&str>,
) -> Result<(), LearnedSparseCorpusError> {
    if case.case_id.trim().is_empty()
        || case.task_id.trim().is_empty()
        || case.query.trim().is_empty()
        || case.tags.is_empty()
        || case.source_ids.is_empty()
    {
        return Err(LearnedSparseCorpusError::InvalidCase {
            case_id: case.case_id.clone(),
            reason: "case identity, query, tags, and sources must be complete".to_string(),
        });
    }
    if let LearnedSparseQueryLanguage::Other(language) = &case.language
        && language.trim().is_empty()
    {
        return Err(LearnedSparseCorpusError::InvalidCase {
            case_id: case.case_id.clone(),
            reason: "custom query language must be non-empty".to_string(),
        });
    }
    case.budget
        .validate(&case.case_id)
        .map_err(|error| LearnedSparseCorpusError::InvalidCase {
            case_id: case.case_id.clone(),
            reason: error.to_string(),
        })?;
    if case
        .source_ids
        .iter()
        .any(|source_id| source_id.trim().is_empty() || !source_ids.contains(source_id.as_str()))
    {
        return Err(LearnedSparseCorpusError::InvalidCase {
            case_id: case.case_id.clone(),
            reason: "case references an unknown source input".to_string(),
        });
    }
    if case.security.is_empty() && case.fidelity != LearnedSparseDataFidelity::RealMaestriaTask {
        return Err(LearnedSparseCorpusError::InvalidCase {
            case_id: case.case_id.clone(),
            reason: "synthetic cases must declare a security or lifecycle scenario".to_string(),
        });
    }
    validate_expectation(case, source_ids)
}

fn validate_expectation(
    case: &LearnedSparseTaskCase,
    source_ids: &BTreeSet<&str>,
) -> Result<(), LearnedSparseCorpusError> {
    match &case.expected {
        LearnedSparseTaskExpectation::Evidence {
            judgments,
            evidence_chain,
            minimum_source_diversity,
            ..
        } => {
            if judgments.is_empty() || evidence_chain.is_empty() || *minimum_source_diversity == 0 {
                return Err(LearnedSparseCorpusError::InvalidCase {
                    case_id: case.case_id.clone(),
                    reason: "evidence expectations require judgments, a chain, and diversity"
                        .to_string(),
                });
            }
            for judgment in judgments {
                if !source_ids.contains(judgment.source_id.as_str())
                    || judgment.source_id.trim().is_empty()
                {
                    return Err(LearnedSparseCorpusError::InvalidCase {
                        case_id: case.case_id.clone(),
                        reason: "evidence judgment references an unknown source".to_string(),
                    });
                }
                for span in &judgment.accepted_spans {
                    span.validate()
                        .map_err(|error| LearnedSparseCorpusError::InvalidCase {
                            case_id: case.case_id.clone(),
                            reason: error.to_string(),
                        })?;
                }
            }
            if evidence_chain.iter().any(|source_id| {
                source_id.trim().is_empty() || !source_ids.contains(source_id.as_str())
            }) {
                return Err(LearnedSparseCorpusError::InvalidCase {
                    case_id: case.case_id.clone(),
                    reason: "evidence chain references an unknown source".to_string(),
                });
            }
        }
        LearnedSparseTaskExpectation::Abstain { .. } => {
            if !case.tags.iter().any(|tag| {
                matches!(
                    tag,
                    LearnedSparseCaseTag::CorrectAbstention | LearnedSparseCaseTag::NoEvidence
                )
            }) {
                return Err(LearnedSparseCorpusError::InvalidCase {
                    case_id: case.case_id.clone(),
                    reason: "abstention cases must carry an abstention or no-evidence tag"
                        .to_string(),
                });
            }
        }
        LearnedSparseTaskExpectation::UnsupportedCapability { capability } => {
            if capability.trim().is_empty() {
                return Err(LearnedSparseCorpusError::InvalidCase {
                    case_id: case.case_id.clone(),
                    reason: "unsupported capability must be named".to_string(),
                });
            }
        }
        LearnedSparseTaskExpectation::Conflict {
            source_ids: conflict_sources,
        } => {
            if conflict_sources.len() < 2
                || conflict_sources
                    .iter()
                    .any(|source_id| !source_ids.contains(source_id.as_str()))
            {
                return Err(LearnedSparseCorpusError::InvalidCase {
                    case_id: case.case_id.clone(),
                    reason: "conflict expectations require two known sources".to_string(),
                });
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LearnedSparseCorpusError {
    #[error("invalid learned-sparse task corpus JSON: {0}")]
    InvalidJson(String),
    #[error("invalid learned-sparse task corpus: {0}")]
    InvalidCorpus(String),
    #[error("invalid learned-sparse source manifest: {0}")]
    InvalidSource(String),
    #[error("invalid learned-sparse judgment guidance: {0}")]
    InvalidGuidance(String),
    #[error("invalid learned-sparse case {case_id}: {reason}")]
    InvalidCase { case_id: String, reason: String },
    #[error("duplicate learned-sparse case {0}")]
    DuplicateCase(String),
}
