use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, EvidenceCandidate,
    EvidenceCoverage, EvidenceId, EvidenceRequirements, EvidenceSpan, FreshnessRequirement,
    FreshnessStatus, IndexGenerationId, Modality, ModalitySet, QueryId, RetrievalModelFingerprint,
    RetrievalReason, RetrievalScoreSet, SearchBudget, SearchIntent, SearchOutcome, SearchPlan,
    SearchStage, SearchStatus, SearchStopReason, SearchTrace, SourceLocation, StopConditions,
    TrustLabel,
};
use maestria_retrieval::golden::{
    GoldenCorpus, GoldenGate, GoldenGateConfig, GoldenJudgment, GoldenObservation, GoldenProfile,
    GoldenQuery, Metric, ResourceMetrics, SecurityMetrics,
};

fn fixture_scores(
    bm25: u32,
    dense: u32,
) -> Result<RetrievalScoreSet, maestria_domain::SearchCompatibilityError> {
    let mut lanes = Vec::new();
    if bm25 != 0 {
        let representation = maestria_domain::RepresentationName::new("lexical_text_v1");
        lanes.push(maestria_domain::RetrievalLaneScore::new(
            maestria_domain::RetrievalScoreKind::LexicalBm25,
            i64::from(bm25),
            maestria_domain::RetrievalRawRank::ranked(1),
            maestria_domain::RetrievalScoreScale::unbounded("fixture_bm25"),
            representation.clone(),
            maestria_domain::RetrievalScoreFingerprint::new(
                maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:lexical-bm25:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        ));
    }
    if dense != 0 {
        let representation = maestria_domain::RepresentationName::new("dense_text_v1");
        lanes.push(maestria_domain::RetrievalLaneScore::new(
            maestria_domain::RetrievalScoreKind::DenseSimilarity,
            i64::from(dense),
            maestria_domain::RetrievalRawRank::ranked(1),
            maestria_domain::RetrievalScoreScale::bounded_fixed_point(
                "fixture_dense_micros",
                1_000_000,
                0,
                1_000_000,
            ),
            representation.clone(),
            maestria_domain::RetrievalScoreFingerprint::new(
                maestria_domain::RetrievalModelFingerprint::new(
                    "fixture:dense-similarity:v1".to_string(),
                )?,
                std::collections::BTreeMap::from([(
                    "representation".to_string(),
                    representation.0,
                )]),
            ),
        ));
    }
    RetrievalScoreSet::new(lanes)
}

fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(7),
        original_query: "alpha".to_owned(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(11),
        index_generation: IndexGenerationId::new(13),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(1000, 1000)?,
        stop_conditions: StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: vec![],
            required_subquestions: vec![],
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("trace:v1".to_owned())?,
        original_intent: None,
        route_decision: None,
    })
}

fn candidate(id: u64, start: u32) -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate {
        coverage_keys: vec![],
        evidence_id: EvidenceId::new(id),
        artifact_version: ArtifactVersionId::new(100 + id),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: "notes.md".to_owned(),
                start_line: start,
                end_line: start,
            },
            ContentRange { start: 0, end: 5 },
        )?,
        scores: fixture_scores(100 - id as u32, 0)?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::ExactMatch],
    })
}

fn observation(
    plan: &SearchPlan,
    evidence: Vec<EvidenceCandidate>,
    status: SearchStatus,
) -> Result<GoldenObservation, Box<dyn std::error::Error>> {
    observation_with_profile(plan, evidence, status, GoldenProfile::V0_4)
}

fn observation_with_profile(
    plan: &SearchPlan,
    evidence: Vec<EvidenceCandidate>,
    status: SearchStatus,
    profile: GoldenProfile,
) -> Result<GoldenObservation, Box<dyn std::error::Error>> {
    let evidence_empty = evidence.is_empty();
    let stop_reason = match &status {
        SearchStatus::Abstained => SearchStopReason::Abstained,
        SearchStatus::NoEvidenceFound => SearchStopReason::NoEvidence,
        SearchStatus::DeniedByPolicy | SearchStatus::QuarantinedForReview => {
            SearchStopReason::PolicyDenied
        }
        SearchStatus::EvidenceIncomplete
        | SearchStatus::StaleEvidenceOnly
        | SearchStatus::SourcesConflict => SearchStopReason::RequirementsUnmet,
        _ if evidence.len() >= plan.stop_conditions.max_results as usize => {
            SearchStopReason::ResultsLimit
        }
        _ => SearchStopReason::EvidenceComplete,
    };
    let trace = SearchTrace::from_plan(
        plan,
        vec!["exact".to_owned()],
        &evidence,
        vec![],
        Some("rrf-fixed-k60".to_owned()),
        vec![],
        stop_reason,
    );
    Ok(GoldenObservation {
        query_id: plan.query_id,
        profile,
        outcome: SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: plan.fingerprint.clone(),
            index_generation: plan.index_generation,
            status,
            evidence,
            coverage: EvidenceCoverage {
                required_claims: vec![],
                required_subquestions: vec![],
                distinct_sources: 0,
                distinct_documents: 0,
                distinct_sections: 0,
                candidate_coverage_keys: vec![],
                percent_covered: if evidence_empty { 0 } else { 100 },
                gaps_identified: vec![],
            },
            conflicts: vec![],
        },
        resources: ResourceMetrics {
            latency_ms: 4,
            memory_bytes: 100,
            disk_bytes: 200,
            ingest_update_ms: None,
            energy_millijoules: None,
            telemetry_complete: true,
        },
        security: SecurityMetrics::measured(),
    })
}

fn corpus(
    plan: &SearchPlan,
    judgments: Vec<GoldenJudgment>,
) -> Result<GoldenCorpus, Box<dyn std::error::Error>> {
    Ok(GoldenCorpus {
        schema_version: GoldenGate::CURRENT_SCHEMA_VERSION,
        corpus_snapshot: plan.corpus_snapshot,
        index_generation: plan.index_generation,
        fingerprint: plan.fingerprint.clone(),
        queries: vec![GoldenQuery {
            query_id: plan.query_id,
            expected_plan: plan.clone(),
            expected_status: SearchStatus::Answerable,
            original_query: plan.original_query.clone(),
            judgments,
            expected_trace: None,
        }],
    })
}

fn permissive_gate() -> GoldenGate {
    GoldenGate {
        k: 10,
        config: GoldenGateConfig {
            profile: maestria_retrieval::golden::GoldenProfile::V0_4,
            min_recall_at_k: Metric::ZERO,
            min_ndcg_at_k: Metric::ZERO,
            min_mrr: Metric::ZERO,
            min_exact_span_recall: Metric::ZERO,
            min_material_quality_delta: Metric::ZERO,
            max_latency_ms: 100,
            max_memory_bytes: 1000,
            max_disk_bytes: 1000,
            max_ingest_update_ms: None,
            max_energy_millijoules: None,
            max_acl_leakage: 0,
            max_attack_successes: 0,
            max_privacy_violations: 0,
        },
    }
}

#[test]
fn golden_gate_reports_relevance_and_exact_span_metrics() -> Result<(), Box<dyn std::error::Error>>
{
    let plan = plan()?;
    let first = candidate(1, 3)?;
    let second = candidate(2, 4)?;
    let corpus = corpus(
        &plan,
        vec![
            GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 3,
                exact_span: Some(first.source_span.clone()),
            },
            GoldenJudgment {
                evidence_id: second.evidence_id,
                relevance: 1,
                exact_span: None,
            },
        ],
    )?;
    let reports = permissive_gate().evaluate(
        &corpus,
        &[observation(
            &plan,
            vec![first, second],
            SearchStatus::Answerable,
        )?],
    )?;
    assert_eq!(reports[0].recall_at_k[&10], Metric::ONE);
    assert_eq!(reports[0].mrr, Metric::ONE);
    assert_eq!(reports[0].exact_span_recall, Metric::ONE);
    Ok(())
}

#[test]
fn golden_metrics_do_not_count_duplicate_evidence_twice() -> Result<(), Box<dyn std::error::Error>>
{
    let plan = plan()?;
    let first = candidate(1, 3)?;
    let second = candidate(2, 4)?;
    let corpus = corpus(
        &plan,
        vec![
            GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 1,
                exact_span: None,
            },
            GoldenJudgment {
                evidence_id: second.evidence_id,
                relevance: 1,
                exact_span: None,
            },
        ],
    )?;
    let report = permissive_gate().evaluate(
        &corpus,
        &[observation(
            &plan,
            vec![first.clone(), first],
            SearchStatus::Answerable,
        )?],
    )?;
    assert_eq!(report[0].recall_at_k[&10], Metric::from_ratio(1, 2));
    Ok(())
}

#[test]
fn golden_gate_rejects_security_regressions() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let first = candidate(1, 3)?;
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: first.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    )?;
    let mut observation = observation(&plan, vec![first], SearchStatus::Answerable)?;
    observation.security.acl_leakage = 1;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation])
        .err()
        .ok_or("ACL leakage must fail the gate")?;
    assert!(error.to_string().contains("ACL leakage"));
    Ok(())
}

#[test]
fn golden_gate_keeps_abstention_as_a_measurable_empty_result()
-> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: EvidenceId::new(1),
            relevance: 1,
            exact_span: None,
        }],
    )?;
    corpus.queries[0].expected_status = SearchStatus::Abstained;

    let report = permissive_gate().evaluate(
        &corpus,
        &[observation(&plan, vec![], SearchStatus::Abstained)?],
    )?;
    assert_eq!(report[0].recall_at_k[&10], Metric::ZERO);
    assert_eq!(report[0].mrr, Metric::ZERO);
    assert_eq!(report[0].ndcg_at_k[&10], Metric::ZERO);
    assert_eq!(report[0].exact_span_recall, Metric::ONE);
    Ok(())
}

#[test]
fn golden_gate_accepts_expected_no_evidence_query() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut corpus = corpus(&plan, vec![])?;
    corpus.queries[0].expected_status = SearchStatus::NoEvidenceFound;
    let report = permissive_gate().evaluate(
        &corpus,
        &[observation(&plan, vec![], SearchStatus::NoEvidenceFound)?],
    )?;
    assert_eq!(report[0].recall_at_k[&10], Metric::ONE);
    assert_eq!(report[0].mrr, Metric::ZERO);
    Ok(())
}

#[test]
fn golden_gate_rejects_resource_and_attack_regressions() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let first = candidate(1, 3)?;
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: first.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    )?;
    let mut observation = observation(&plan, vec![first], SearchStatus::Answerable)?;
    observation.resources.latency_ms = 101;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation.clone()])
        .err()
        .ok_or("latency must fail the gate")?;
    assert!(error.to_string().contains("latency"));
    observation.resources.latency_ms = 4;
    observation.resources.memory_bytes = 1001;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation.clone()])
        .err()
        .ok_or("memory must fail the gate")?;
    assert!(error.to_string().contains("memory"));

    observation.resources.memory_bytes = 100;
    observation.resources.disk_bytes = 1001;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation.clone()])
        .err()
        .ok_or("disk must fail the gate")?;
    observation.resources.disk_bytes = 200;
    assert!(error.to_string().contains("disk"));

    observation.resources.latency_ms = 4;
    observation.security.attack_successes = 1;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation])
        .err()
        .ok_or("attack success must fail the gate")?;
    assert!(error.to_string().contains("attack success"));
    Ok(())
}

#[test]
fn golden_gate_rejects_configured_quality_regressions() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: EvidenceId::new(1),
            relevance: 1,
            exact_span: Some(candidate(1, 3)?.source_span.clone()),
        }],
    )?;
    for (field, expected_reason) in [
        ("recall", "Recall@k"),
        ("ndcg", "nDCG@k"),
        ("mrr", "MRR"),
        ("exact", "exact-span recall"),
    ] {
        let mut gate = permissive_gate();
        match field {
            "recall" => gate.config.min_recall_at_k = Metric::ONE,
            "ndcg" => gate.config.min_ndcg_at_k = Metric::ONE,
            "mrr" => gate.config.min_mrr = Metric::ONE,
            "exact" => gate.config.min_exact_span_recall = Metric::ONE,
            _ => return Err(std::io::Error::other("unexpected quality field").into()),
        }
        let error = gate
            .evaluate(
                &corpus,
                &[observation(&plan, vec![], SearchStatus::Answerable)?],
            )
            .err()
            .ok_or("quality threshold must fail")?;
        assert!(error.to_string().contains(expected_reason));
    }
    Ok(())
}

#[test]
fn golden_gate_rejects_invalid_corpus_shapes() -> Result<(), Box<dyn std::error::Error>> {
    let plan = plan()?;
    let mut empty = corpus(&plan, vec![])?;
    empty.queries.clear();
    assert!(matches!(
        permissive_gate().evaluate(&empty, &[]),
        Err(maestria_retrieval::golden::GoldenGateError::EmptyCorpus)
    ));

    let mut invalid_k = permissive_gate();
    invalid_k.k = 0;
    let nonempty = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: EvidenceId::new(1),
            relevance: 1,
            exact_span: None,
        }],
    )?;
    assert!(matches!(
        invalid_k.evaluate(&nonempty, &[]),
        Err(maestria_retrieval::golden::GoldenGateError::InvalidK)
    ));

    let mut duplicate_query = nonempty.clone();
    duplicate_query
        .queries
        .push(duplicate_query.queries[0].clone());
    assert!(matches!(
        permissive_gate().evaluate(&duplicate_query, &[]),
        Err(maestria_retrieval::golden::GoldenGateError::DuplicateQuery(
            _
        ))
    ));

    let mut duplicate_judgment = nonempty;
    let judgment = duplicate_judgment.queries[0].judgments[0].clone();
    duplicate_judgment.queries[0].judgments.push(judgment);
    assert!(matches!(
        permissive_gate().evaluate(&duplicate_judgment, &[]),
        Err(maestria_retrieval::golden::GoldenGateError::DuplicateJudgment { .. })
    ));
    Ok(())
}

fn comparison_config(profile: GoldenProfile) -> GoldenGateConfig {
    GoldenGateConfig {
        profile,
        min_recall_at_k: Metric::ZERO,
        min_ndcg_at_k: Metric::ZERO,
        min_mrr: Metric::ZERO,
        min_exact_span_recall: Metric::ZERO,
        min_material_quality_delta: Metric::MATERIAL_QUALITY_DELTA,
        max_latency_ms: 100,
        max_memory_bytes: 1_000,
        max_disk_bytes: 1_000,
        max_ingest_update_ms: Some(100),
        max_energy_millijoules: Some(1_000),
        max_acl_leakage: 0,
        max_attack_successes: 0,
        max_privacy_violations: 0,
    }
}

#[test]
fn golden_comparison_promotes_only_for_material_quality_improvement()
-> Result<(), Box<dyn std::error::Error>> {
    use maestria_retrieval::golden::{
        GoldenComparison, GoldenProfile, PromotionDecision, PromotionRecord,
    };
    let plan = plan()?;
    let first = candidate(1, 0)?;
    let second = candidate(2, 1)?;
    let corpus = corpus(
        &plan,
        vec![
            GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 1,
                exact_span: None,
            },
            GoldenJudgment {
                evidence_id: second.evidence_id,
                relevance: 1,
                exact_span: None,
            },
        ],
    )?;
    let mut baseline_obs = observation(&plan, vec![first.clone()], SearchStatus::Answerable)?;
    baseline_obs.resources.ingest_update_ms = Some(8);
    baseline_obs.resources.energy_millijoules = Some(12);
    let mut candidate_obs = observation_with_profile(
        &plan,
        vec![first, second],
        SearchStatus::Answerable,
        GoldenProfile::V0_5,
    )?;
    candidate_obs.resources.ingest_update_ms = Some(8);
    candidate_obs.resources.energy_millijoules = Some(12);
    let result = GoldenComparison {
        k: 10,
        tier: maestria_retrieval::golden::BackendTier::Small,
        workload: "golden-gate-tests".to_string(),
    }
    .compare(
        &corpus,
        &comparison_config(GoldenProfile::V0_4),
        &[baseline_obs],
        &comparison_config(GoldenProfile::V0_5),
        &[candidate_obs],
        Some(PromotionRecord {
            evaluation_id: "eval_id_123".to_string(),
            evaluation_date: "2026-07-16".to_string(),
        }),
    )?;
    assert_eq!(
        result.report.backend_tier,
        maestria_retrieval::golden::BackendTier::Small
    );
    assert_eq!(result.report.workload, "golden-gate-tests");
    assert_eq!(result.report.corpus_snapshot, corpus.corpus_snapshot);
    assert_eq!(result.report.index_generation, corpus.index_generation);
    assert_eq!(result.report.fingerprint, corpus.fingerprint);
    match result.decision {
        PromotionDecision::Promote {
            evaluation_id,
            evaluation_date,
            ..
        } => {
            assert_eq!(evaluation_id, "eval_id_123");
            assert_eq!(evaluation_date, "2026-07-16");
        }
        PromotionDecision::RetainBaseline { reason: _ } => {
            return Err("unexpected retention".into());
        }
    }
    Ok(())
}

#[test]
fn golden_comparison_requires_complete_promotion_telemetry()
-> Result<(), Box<dyn std::error::Error>> {
    use maestria_retrieval::golden::{
        GoldenComparison, GoldenProfile, PromotionDecision, PromotionRecord,
    };
    let plan = plan()?;
    let first = candidate(1, 0)?;
    let second = candidate(2, 1)?;
    let corpus = corpus(
        &plan,
        vec![
            GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 1,
                exact_span: None,
            },
            GoldenJudgment {
                evidence_id: second.evidence_id,
                relevance: 1,
                exact_span: None,
            },
        ],
    )?;
    let result = GoldenComparison {
        k: 10,
        tier: maestria_retrieval::golden::BackendTier::Small,
        workload: "golden-gate-tests".to_string(),
    }
    .compare(
        &corpus,
        &comparison_config(GoldenProfile::V0_4),
        &[observation(
            &plan,
            vec![first.clone()],
            SearchStatus::Answerable,
        )?],
        &comparison_config(GoldenProfile::V0_5),
        &[observation_with_profile(
            &plan,
            vec![first, second],
            SearchStatus::Answerable,
            GoldenProfile::V0_5,
        )?],
        Some(PromotionRecord {
            evaluation_id: "eval_id_telemetry".to_string(),
            evaluation_date: "2026-07-16".to_string(),
        }),
    )?;
    match result.decision {
        PromotionDecision::RetainBaseline { reason } => {
            assert!(reason.contains("complete"));
            assert!(reason.contains("telemetry"));
        }
        PromotionDecision::Promote { .. } => {
            return Err(std::io::Error::other("incomplete telemetry must retain baseline").into());
        }
    }
    Ok(())
}

#[test]
fn golden_comparison_retains_baseline_when_candidate_regresses()
-> Result<(), Box<dyn std::error::Error>> {
    use maestria_retrieval::golden::{GoldenComparison, GoldenProfile, PromotionDecision};
    let plan = plan()?;
    let candidate = candidate(1, 0)?;
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: candidate.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    )?;
    let mut baseline_obs = observation(&plan, vec![candidate.clone()], SearchStatus::Answerable)?;
    baseline_obs.resources.latency_ms = 10;
    let mut candidate_obs = observation_with_profile(
        &plan,
        vec![candidate],
        SearchStatus::Answerable,
        GoldenProfile::V0_5,
    )?;
    candidate_obs.resources.latency_ms = 20;
    let result = GoldenComparison {
        k: 10,
        tier: maestria_retrieval::golden::BackendTier::Small,
        workload: "golden-gate-tests".to_string(),
    }
    .compare(
        &corpus,
        &comparison_config(GoldenProfile::V0_4),
        &[baseline_obs],
        &comparison_config(GoldenProfile::V0_5),
        &[candidate_obs],
        None,
    )?;
    match result.decision {
        PromotionDecision::RetainBaseline { reason } => {
            assert!(reason.contains("p50_latency_ms"));
            assert!(reason.contains("p95_latency_ms"));
            assert!(reason.contains("p99_latency_ms"));
        }
        PromotionDecision::Promote { .. } => {
            return Err(std::io::Error::other("regression must retain baseline").into());
        }
    }
    Ok(())
}
