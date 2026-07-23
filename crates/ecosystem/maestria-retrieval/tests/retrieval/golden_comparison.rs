use maestria_domain::SearchStatus;
use maestria_retrieval::golden::{
    GoldenGateConfig, GoldenProfile, Metric, PromotionDecision, PromotionRecord,
};

use crate::common::golden::{candidate, corpus, observation, observation_with_profile, plan};

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
    let plan = plan()?;
    let first = candidate(1, 0)?;
    let second = candidate(2, 1)?;
    let corpus = corpus(
        &plan,
        vec![
            maestria_retrieval::golden::GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 1,
                exact_span: None,
            },
            maestria_retrieval::golden::GoldenJudgment {
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
    let result = maestria_retrieval::golden::GoldenComparison {
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
    let plan = plan()?;
    let first = candidate(1, 0)?;
    let second = candidate(2, 1)?;
    let corpus = corpus(
        &plan,
        vec![
            maestria_retrieval::golden::GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 1,
                exact_span: None,
            },
            maestria_retrieval::golden::GoldenJudgment {
                evidence_id: second.evidence_id,
                relevance: 1,
                exact_span: None,
            },
        ],
    )?;
    let result = maestria_retrieval::golden::GoldenComparison {
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
    let plan = plan()?;
    let c = candidate(1, 0)?;
    let corpus = corpus(
        &plan,
        vec![maestria_retrieval::golden::GoldenJudgment {
            evidence_id: c.evidence_id,
            relevance: 1,
            exact_span: None,
        }],
    )?;
    let mut baseline_obs = observation(&plan, vec![c.clone()], SearchStatus::Answerable)?;
    baseline_obs.resources.latency_ms = 10;
    let mut candidate_obs = observation_with_profile(
        &plan,
        vec![c],
        SearchStatus::Answerable,
        GoldenProfile::V0_5,
    )?;
    candidate_obs.resources.latency_ms = 20;
    let result = maestria_retrieval::golden::GoldenComparison {
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
