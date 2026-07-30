use maestria_retrieval::{
    LearnedSparseDataFidelity, LearnedSparseDataSplit, LearnedSparseQueryClass,
    LearnedSparseTaskCorpus,
};

const CORPUS: &str = include_str!("../../../../tests/contracts/learned_sparse_task_corpus_v1.json");

#[test]
fn frozen_task_corpus_has_independent_development_and_final_cases()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = LearnedSparseTaskCorpus::from_json(CORPUS)?;
    assert_eq!(corpus.cases.len(), 18);
    assert!(
        corpus
            .cases
            .iter()
            .any(|case| case.split == LearnedSparseDataSplit::Development)
    );
    assert!(
        corpus
            .cases
            .iter()
            .any(|case| case.split == LearnedSparseDataSplit::FinalEvaluation)
    );
    for class in LearnedSparseQueryClass::all() {
        let final_cases = corpus
            .cases
            .iter()
            .filter(|case| {
                case.class == class && case.split == LearnedSparseDataSplit::FinalEvaluation
            })
            .collect::<Vec<_>>();
        assert_eq!(final_cases.len(), 2);
        assert_eq!(
            final_cases
                .iter()
                .map(|case| case.task_id.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2
        );
    }
    assert!(
        corpus
            .cases
            .iter()
            .any(|case| case.fidelity == LearnedSparseDataFidelity::SyntheticAdversarial)
    );
    assert!(
        corpus
            .cases
            .iter()
            .any(|case| case.fidelity == LearnedSparseDataFidelity::SyntheticLifecycle)
    );
    Ok(())
}

#[test]
fn frozen_task_corpus_rejects_invalid_source_and_split_metadata()
-> Result<(), Box<dyn std::error::Error>> {
    let mut corpus = LearnedSparseTaskCorpus::from_json(CORPUS)?;
    corpus.source_inputs[0].path = "../outside-repository.md".to_string();
    assert!(corpus.validate().is_err());

    let mut corpus = LearnedSparseTaskCorpus::from_json(CORPUS)?;
    corpus
        .cases
        .retain(|case| case.case_id != "final-exact-path");
    assert!(corpus.validate().is_err());
    Ok(())
}

#[test]
fn judgment_and_security_expectations_are_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = LearnedSparseTaskCorpus::from_json(CORPUS)?;
    let evidence_cases = corpus
        .cases
        .iter()
        .filter(|case| {
            matches!(
                case.expected,
                maestria_retrieval::LearnedSparseTaskExpectation::Evidence { .. }
            )
        })
        .count();
    let protected_cases = corpus
        .cases
        .iter()
        .filter(|case| !case.security.is_empty())
        .count();
    assert!(evidence_cases >= 10);
    assert!(protected_cases >= 5);
    Ok(())
}
