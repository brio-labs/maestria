use std::collections::{BTreeMap, BTreeSet};

use super::*;
use maestria_domain::{ContentHash, SearchIntent};

#[test]
fn stage_b_is_not_authorized_without_both_independent_decisions() {
    let comparison = LateInteractionStageAComparison {
        corpus_id: "c".into(),
        corpus_revision: "r".into(),
        classes: BTreeMap::new(),
    };
    let decision = decide_stage_b(
        &comparison,
        &IndexedRetrievalNeed::MeasuredNeed {
            case_ids: vec!["a".into(), "b".into()],
            missing_relevant_count: 1,
            source_encoding_bottleneck_cases: Vec::new(),
        },
    );
    assert!(matches!(
        decision,
        LateInteractionStageBDecision::NotAuthorized { .. }
    ));
}
#[test]
fn frozen_corpus_uses_strict_source_and_judgment_schema()
-> Result<(), LateInteractionBenchmarkError> {
    let input = include_str!("../../../../tests/contracts/late_interaction_task_corpus_v1.json");
    let corpus = LateInteractionBenchmarkCorpus::from_json(input)?;
    assert_eq!(corpus.cases.len(), 10);
    assert!(!corpus.source_paths.is_empty());
    assert!(corpus.cases.iter().all(|case| !case.source_file.is_empty()));
    Ok(())
}

fn quality(value: u32) -> LateInteractionQualityMetrics {
    let measured = || Measurement::measured(value);
    LateInteractionQualityMetrics {
        recall_at_5: measured(),
        recall_at_20: measured(),
        recall_at_50: measured(),
        recall_at_100: measured(),
        ndcg_at_10: measured(),
        ndcg_at_20: measured(),
        mrr_at_10: measured(),
        exact_span_recall: measured(),
        constraint_satisfaction: measured(),
    }
}

fn observation(
    corpus: &LateInteractionBenchmarkCorpus,
    case: &LateInteractionBenchmarkCase,
    route: LateInteractionRoute,
    value: u32,
) -> Result<LateInteractionObservation, LateInteractionBenchmarkError> {
    Ok(LateInteractionObservation {
        corpus_id: corpus.corpus_id.clone(),
        corpus_revision: corpus.revision.clone(),
        case_id: case.case_id.clone(),
        query_class: case.query_class,
        route,
        candidate_input_hash: ContentHash::new(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        )
        .map_err(|_| LateInteractionBenchmarkError::InvalidCorpus("hash"))?,
        quality: quality(value),
        resources: LateInteractionResourceMetrics {
            end_to_end_p50_ms: Measurement::measured(1),
            end_to_end_p95_ms: Measurement::measured(1),
            scorer_p95_ms: Measurement::measured(1),
            peak_memory_bytes: Measurement::measured(1),
            model_storage_bytes: Measurement::measured(1),
            index_storage_bytes: Measurement::measured(1),
            energy_millijoules: Measurement::measured(1),
        },
        safety: LateInteractionSafetyMetrics {
            acl_leaks: 0,
            secret_exposure: 0,
            quarantine_escape: 0,
            prompt_injection_fail_open: 0,
            protected_provider_calls: 0,
        },
        measurement_status: Measurement::measured(()),
    })
}

fn stage_a_fingerprints(
    corpus: &LateInteractionBenchmarkCorpus,
) -> LateInteractionStageAFingerprints {
    LateInteractionStageAFingerprints {
        corpus_snapshot: "snapshot".to_string(),
        index_generation: "generation".to_string(),
        identity_digest: "identity".to_string(),
        profile_digest: "profile".to_string(),
        scorer_fingerprint: "scorer".to_string(),
        source_hash: corpus.source_hash.as_str().to_string(),
        judgment_hash: corpus.judgment_hash.as_str().to_string(),
    }
}

fn assert_stage_b_report_hash_bound(
    corpus: &LateInteractionBenchmarkCorpus,
    observations: Vec<LateInteractionObservation>,
) -> Result<(), LateInteractionBenchmarkError> {
    let stage_a = LateInteractionStageAReport::from_observations(
        corpus,
        "2026-09-09",
        "aggregation-stage-a",
        "real",
        LateInteractionStageAFingerprints {
            corpus_snapshot: "snapshot".to_string(),
            index_generation: "generation".to_string(),
            identity_digest: "identity".to_string(),
            profile_digest: "profile".to_string(),
            scorer_fingerprint: "scorer".to_string(),
            source_hash: corpus.source_hash.as_str().to_string(),
            judgment_hash: corpus.judgment_hash.as_str().to_string(),
        },
        observations,
    )?;
    let stage_b = LateInteractionStageBReport::from_stage_a(
        &stage_a,
        corpus,
        "2026-09-09",
        "aggregation-stage-b",
        IndexedRetrievalNeed::Unavailable {
            reason: "indexed retrieval need was not measured".to_string(),
        },
    )?;
    assert!(matches!(
        stage_b.decision,
        LateInteractionStageBDecision::NotAuthorized { .. }
    ));
    assert!(matches!(
        LateInteractionStageBReport::from_stage_a(
            &stage_a,
            corpus,
            "2026-09-09",
            "invalid-indexed-need",
            IndexedRetrievalNeed::MeasuredNeed {
                case_ids: vec!["a".to_string(), "missing".to_string()],
                missing_relevant_count: 1,
                source_encoding_bottleneck_cases: Vec::new(),
            },
        ),
        Err(LateInteractionBenchmarkError::InvalidReport(_))
    ));
    let mut serialized = serde_json::to_value(&stage_b)
        .map_err(|_| LateInteractionBenchmarkError::InvalidCorpus("report"))?;
    let object = serialized
        .as_object_mut()
        .ok_or(LateInteractionBenchmarkError::InvalidCorpus(
            "report object",
        ))?;
    object.insert("unexpected".to_string(), serde_json::Value::from(true));
    assert!(serde_json::from_value::<LateInteractionStageBReport>(serialized).is_err());
    let mut tampered = stage_b;
    tampered.stage_a_report_hash = ContentHash::new(
        "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_string(),
    )
    .map_err(|_| LateInteractionBenchmarkError::InvalidCorpus("hash"))?;
    assert!(matches!(
        tampered.validate_against_stage_a(&stage_a, corpus),
        Err(LateInteractionBenchmarkError::InvalidReport(_))
            | Err(LateInteractionBenchmarkError::ReportIdentityMismatch)
    ));
    Ok(())
}

fn assert_incomplete_stage_a_is_not_promotable(
    corpus: &LateInteractionBenchmarkCorpus,
    observations: &[LateInteractionObservation],
) -> Result<(), LateInteractionBenchmarkError> {
    let mut incomplete = observations.to_vec();
    for item in &mut incomplete {
        if item.route == LateInteractionRoute::LateInteractionReranker {
            item.resources.peak_memory_bytes =
                Measurement::unavailable("resident memory was not measured");
        }
    }
    let report = LateInteractionStageAReport::from_observations(
        corpus,
        "2026-09-09",
        "incomplete-stage-a",
        "real",
        stage_a_fingerprints(corpus),
        incomplete,
    )?;
    assert!(!report.promotion.authorized);
    assert!(matches!(
        report.decisions.get("FactualLocal"),
        Some(decision) if decision == "RetainBaseline"
    ));
    Ok(())
}

#[test]
fn stage_a_aggregates_multiple_cases_per_query_class_and_route()
-> Result<(), LateInteractionBenchmarkError> {
    let case = |case_id: &str, source_file: &str| LateInteractionBenchmarkCase {
        case_id: case_id.to_string(),
        query: format!("query {case_id}"),
        query_class: SearchIntent::FactualLocal,
        tags: BTreeSet::new(),
        relevant_evidence_ids: BTreeSet::new(),
        source_file: source_file.to_string(),
        judgments: vec![LateInteractionBenchmarkJudgment {
            source: source_file.to_string(),
            grade: 1,
            start: 0,
            end: 1,
        }],
        latency_budget_ms: 250,
    };
    let corpus = LateInteractionBenchmarkCorpus {
        schema_version: LATE_INTERACTION_BENCHMARK_SCHEMA_VERSION,
        corpus_id: "aggregation".to_string(),
        revision: "v1".to_string(),
        judgment_set: "judgments".to_string(),
        source_hash: ContentHash::new(
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        )
        .map_err(|_| LateInteractionBenchmarkError::InvalidCorpus("hash"))?,
        judgment_hash: ContentHash::new(
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        )
        .map_err(|_| LateInteractionBenchmarkError::InvalidCorpus("hash"))?,
        source_paths: vec!["a.txt".to_string(), "b.txt".to_string()],
        cases: vec![case("a", "a.txt"), case("b", "b.txt")],
    };
    let mut observations = Vec::new();
    for (index, case) in corpus.cases.iter().enumerate() {
        for route in LateInteractionRoute::all_stage_a() {
            let value = if route == LateInteractionRoute::LateInteractionReranker {
                if index == 0 { 1_000_000 } else { 500_000 }
            } else {
                500_000
            };
            observations.push(observation(&corpus, case, route, value)?);
        }
    }
    let comparison = LateInteractionStageAComparison::evaluate(&corpus, &observations)?;
    let class = comparison.classes.get(&SearchIntent::FactualLocal).ok_or(
        LateInteractionBenchmarkError::MissingClass(SearchIntent::FactualLocal),
    )?;
    assert_eq!(class.reranker.ndcg_at_10, Measurement::Measured(750_000));
    assert!(matches!(
        class.decision,
        LateInteractionClassDecision::RetainLateInteractionReranker { .. }
    ));
    assert_incomplete_stage_a_is_not_promotable(&corpus, &observations)?;
    assert_stage_b_report_hash_bound(&corpus, observations.clone())?;
    let stage_a = LateInteractionStageAReport::from_observations(
        &corpus,
        "2026-09-09",
        "promotion-stage-a",
        "real",
        stage_a_fingerprints(&corpus),
        observations,
    )?;
    let record = LateInteractionStageAPromotionRecord::from_stage_a(
        &stage_a,
        &corpus,
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "generation",
        "snapshot",
        "lexical-generation",
    )?;
    assert_eq!(
        record.promoted_classes,
        BTreeSet::from([SearchIntent::FactualLocal])
    );
    let mut tampered = record;
    tampered.stage_a_report_hash = ContentHash::new(
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
    )
    .map_err(|_| LateInteractionBenchmarkError::InvalidCorpus("hash"))?;
    assert!(
        tampered
            .validate_against_stage_a(&stage_a, &corpus)
            .is_err()
    );
    Ok(())
}
