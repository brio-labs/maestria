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

fn plan() -> SearchPlan {
    SearchPlan {
        query_id: QueryId::new(7),
        original_query: "alpha".to_owned(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(11),
        index_generation: IndexGenerationId::new(13),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(1000, 1000).expect("valid budget"),
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
        fingerprint: RetrievalModelFingerprint::new("trace:v1".to_owned())
            .expect("valid fingerprint"),
    }
}

fn candidate(id: u64, start: u32) -> EvidenceCandidate {
    EvidenceCandidate {
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
        )
        .expect("valid span"),
        scores: RetrievalScoreSet {
            bm25: 100 - id as u32,
            semantic_similarity: 0,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::ExactMatch],
    }
}

fn observation(
    plan: &SearchPlan,
    evidence: Vec<EvidenceCandidate>,
    status: SearchStatus,
) -> GoldenObservation {
    observation_with_profile(plan, evidence, status, GoldenProfile::V0_4)
}

fn observation_with_profile(
    plan: &SearchPlan,
    evidence: Vec<EvidenceCandidate>,
    status: SearchStatus,
    profile: GoldenProfile,
) -> GoldenObservation {
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
    GoldenObservation {
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
    }
}

fn corpus(plan: &SearchPlan, judgments: Vec<GoldenJudgment>) -> GoldenCorpus {
    GoldenCorpus {
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
        }],
    }
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
fn golden_gate_reports_relevance_and_exact_span_metrics() {
    let plan = plan();
    let first = candidate(1, 3);
    let second = candidate(2, 4);
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
    );
    let reports = permissive_gate()
        .evaluate(
            &corpus,
            &[observation(
                &plan,
                vec![first, second],
                SearchStatus::Answerable,
            )],
        )
        .expect("golden gate passes");
    assert_eq!(reports[0].recall_at_k[&10], Metric::ONE);
    assert_eq!(reports[0].mrr, Metric::ONE);
    assert_eq!(reports[0].exact_span_recall, Metric::ONE);
}

#[test]
fn golden_metrics_do_not_count_duplicate_evidence_twice() {
    let plan = plan();
    let first = candidate(1, 3);
    let second = candidate(2, 4);
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
    );
    let report = permissive_gate()
        .evaluate(
            &corpus,
            &[observation(
                &plan,
                vec![first.clone(), first],
                SearchStatus::Answerable,
            )],
        )
        .expect("permissive gate passes");
    assert_eq!(report[0].recall_at_k[&10], Metric::from_ratio(1, 2));
}

#[test]
fn golden_gate_rejects_security_regressions() {
    let plan = plan();
    let first = candidate(1, 3);
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: first.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    );
    let mut observation = observation(&plan, vec![first], SearchStatus::Answerable);
    observation.security.acl_leakage = 1;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation])
        .expect_err("ACL leakage must fail the gate");
    assert!(error.to_string().contains("ACL leakage"));
}

#[test]
fn golden_gate_keeps_abstention_as_a_measurable_empty_result() {
    let plan = plan();
    let mut corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: EvidenceId::new(1),
            relevance: 1,
            exact_span: None,
        }],
    );
    corpus.queries[0].expected_status = SearchStatus::Abstained;

    let report = permissive_gate()
        .evaluate(
            &corpus,
            &[observation(&plan, vec![], SearchStatus::Abstained)],
        )
        .expect("permissive gate records abstention");
    assert_eq!(report[0].recall_at_k[&10], Metric::ZERO);
    assert_eq!(report[0].mrr, Metric::ZERO);
    assert_eq!(report[0].ndcg_at_k[&10], Metric::ZERO);
    assert_eq!(report[0].exact_span_recall, Metric::ONE);
}

#[test]
fn golden_gate_accepts_expected_no_evidence_query() {
    let plan = plan();
    let mut corpus = corpus(&plan, vec![]);
    corpus.queries[0].expected_status = SearchStatus::NoEvidenceFound;
    let report = permissive_gate()
        .evaluate(
            &corpus,
            &[observation(&plan, vec![], SearchStatus::NoEvidenceFound)],
        )
        .expect("expected no-evidence result passes");
    assert_eq!(report[0].recall_at_k[&10], Metric::ONE);
    assert_eq!(report[0].mrr, Metric::ZERO);
}

#[test]
fn golden_gate_rejects_resource_and_attack_regressions() {
    let plan = plan();
    let first = candidate(1, 3);
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: first.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    );
    let mut observation = observation(&plan, vec![first], SearchStatus::Answerable);
    observation.resources.latency_ms = 101;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation.clone()])
        .expect_err("latency must fail the gate");
    assert!(error.to_string().contains("latency"));
    observation.resources.latency_ms = 4;
    observation.resources.memory_bytes = 1001;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation.clone()])
        .expect_err("memory must fail the gate");
    assert!(error.to_string().contains("memory"));

    observation.resources.memory_bytes = 100;
    observation.resources.disk_bytes = 1001;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation.clone()])
        .expect_err("disk must fail the gate");
    observation.resources.disk_bytes = 200;
    assert!(error.to_string().contains("disk"));

    observation.resources.latency_ms = 4;
    observation.security.attack_successes = 1;
    let error = permissive_gate()
        .evaluate(&corpus, &[observation])
        .expect_err("attack success must fail the gate");
    assert!(error.to_string().contains("attack success"));
}

#[test]
fn golden_gate_rejects_configured_quality_regressions() {
    let plan = plan();
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: EvidenceId::new(1),
            relevance: 1,
            exact_span: Some(candidate(1, 3).source_span),
        }],
    );
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
            _ => unreachable!("test case is exhaustive"),
        }
        let error = gate
            .evaluate(
                &corpus,
                &[observation(&plan, vec![], SearchStatus::Answerable)],
            )
            .expect_err("quality threshold must fail");
        assert!(error.to_string().contains(expected_reason));
    }
}

#[test]
fn golden_gate_rejects_invalid_corpus_shapes() {
    let plan = plan();
    let mut empty = corpus(&plan, vec![]);
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
    );
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
fn golden_comparison_promotes_only_for_material_quality_improvement() {
    use maestria_retrieval::golden::{
        GoldenComparison, GoldenProfile, PromotionDecision, PromotionRecord,
    };
    let plan = plan();
    let first = candidate(1, 0);
    let second = candidate(2, 1);
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
    );
    let mut baseline_obs = observation(&plan, vec![first.clone()], SearchStatus::Answerable);
    baseline_obs.resources.ingest_update_ms = Some(8);
    baseline_obs.resources.energy_millijoules = Some(12);
    let mut candidate_obs = observation_with_profile(
        &plan,
        vec![first, second],
        SearchStatus::Answerable,
        GoldenProfile::V0_5,
    );
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
    )
    .expect("comparison should evaluate");
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
        PromotionDecision::RetainBaseline { reason } => panic!("unexpected retention: {reason}"),
    }
}

#[test]
fn golden_comparison_requires_complete_promotion_telemetry() {
    use maestria_retrieval::golden::{
        GoldenComparison, GoldenProfile, PromotionDecision, PromotionRecord,
    };
    let plan = plan();
    let first = candidate(1, 0);
    let second = candidate(2, 1);
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
    );
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
        )],
        &comparison_config(GoldenProfile::V0_5),
        &[observation_with_profile(
            &plan,
            vec![first, second],
            SearchStatus::Answerable,
            GoldenProfile::V0_5,
        )],
        Some(PromotionRecord {
            evaluation_id: "eval_id_telemetry".to_string(),
            evaluation_date: "2026-07-16".to_string(),
        }),
    )
    .expect("comparison should evaluate");
    match result.decision {
        PromotionDecision::RetainBaseline { reason } => {
            assert!(reason.contains("complete"));
            assert!(reason.contains("telemetry"));
        }
        PromotionDecision::Promote { .. } => panic!("incomplete telemetry must retain baseline"),
    }
}

#[test]
fn golden_comparison_retains_baseline_when_candidate_regresses() {
    use maestria_retrieval::golden::{GoldenComparison, GoldenProfile, PromotionDecision};
    let plan = plan();
    let candidate = candidate(1, 0);
    let corpus = corpus(
        &plan,
        vec![GoldenJudgment {
            evidence_id: candidate.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    );
    let mut baseline_obs = observation(&plan, vec![candidate.clone()], SearchStatus::Answerable);
    baseline_obs.resources.latency_ms = 10;
    let mut candidate_obs = observation_with_profile(
        &plan,
        vec![candidate],
        SearchStatus::Answerable,
        GoldenProfile::V0_5,
    );
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
    )
    .expect("comparison should evaluate");
    match result.decision {
        PromotionDecision::RetainBaseline { reason } => {
            assert!(reason.contains("p50_latency_ms"));
            assert!(reason.contains("p95_latency_ms"));
            assert!(reason.contains("p99_latency_ms"));
        }
        PromotionDecision::Promote { .. } => panic!("regression must retain baseline"),
    }
}
