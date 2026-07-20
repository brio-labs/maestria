use std::{collections::BTreeSet, fs};

use maestria_retrieval::golden::Metric;
use maestria_retrieval::{
    VisualBenchmarkCase, VisualBenchmarkComparison, VisualBenchmarkCorpus, VisualBenchmarkError,
    VisualBenchmarkObservation, VisualExecutionPolicy, VisualProviderStatus,
    VisualProviderUnavailableExecutor, VisualQueryClass, VisualRoute, VisualTextLayoutExecutor,
    run_visual_benchmark,
};

fn corpus() -> Result<VisualBenchmarkCorpus, VisualBenchmarkError> {
    VisualBenchmarkCorpus::from_json(include_str!("fixtures/visual-retrieval-benchmark-v1.json"))
}

fn metric(value: u32) -> Result<Metric, &'static str> {
    Metric::new(value).ok_or("metric is outside the fixed-point range")
}

struct Profile {
    page_region_recall: Metric,
    ndcg_at_10: Metric,
    citation_alignment: Metric,
    latency_ms: u64,
    memory_bytes: u64,
    disk_bytes: u64,
    energy_millijoules: u64,
    privacy_violations: u32,
    security_violations: u32,
}

fn profile(
    case: &VisualBenchmarkCase,
    route: VisualRoute,
) -> Result<Profile, Box<dyn std::error::Error>> {
    let winning = matches!(
        case.class,
        VisualQueryClass::Table | VisualQueryClass::Chart | VisualQueryClass::Figure
    );
    let values = match (route, winning) {
        (VisualRoute::TextLayout, _) => (5_000, 6_000, 8_000, 100, 500_000, 1_000_000, 500, 0, 0),
        (VisualRoute::Visual, true) => (9_000, 9_000, 8_500, 130, 1_000_000, 2_000_000, 900, 0, 0),
        (VisualRoute::Visual, false) => (5_000, 6_000, 8_000, 140, 1_000_000, 2_000_000, 900, 0, 0),
    };
    Ok(Profile {
        page_region_recall: metric(values.0)?,
        ndcg_at_10: metric(values.1)?,
        citation_alignment: metric(values.2)?,
        latency_ms: values.3,
        memory_bytes: values.4,
        disk_bytes: values.5,
        energy_millijoules: values.6,
        privacy_violations: values.7,
        security_violations: values.8,
    })
}

fn observations(
    corpus: &VisualBenchmarkCorpus,
) -> Result<Vec<VisualBenchmarkObservation>, Box<dyn std::error::Error>> {
    let corpus_id = corpus.corpus_id.clone();
    let corpus_revision = corpus.corpus_revision.clone();
    let evaluation_date = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "unknown".into(),
    };
    Ok(run_visual_benchmark(corpus, &move |case, route| {
        let profile = profile(&case, route)
            .map_err(|error| VisualBenchmarkError::InvalidCorpus(error.to_string()))?;
        Ok(VisualBenchmarkObservation {
            corpus_id: corpus_id.clone(),
            corpus_revision: corpus_revision.clone(),
            evaluation_date: evaluation_date.clone(),
            model_fingerprint: "test-profile-v1".into(),
            provider_config: serde_json::Value::Object(serde_json::Map::from_iter([(
                "provider".to_string(),
                serde_json::Value::String("test_profile".to_string()),
            )])),
            case_id: case.case_id,
            route,
            page_region_recall: profile.page_region_recall,
            ndcg_at_10: profile.ndcg_at_10,
            citation_alignment: profile.citation_alignment,
            latency_ms: profile.latency_ms,
            memory_bytes: profile.memory_bytes,
            disk_bytes: profile.disk_bytes,
            energy_millijoules: profile.energy_millijoules,
            privacy_violations: profile.privacy_violations,
            security_violations: profile.security_violations,
            provider_status: VisualProviderStatus::Available,
        })
    })?)
}

#[test]
fn visual_fixture_covers_all_query_classes_and_page_region_judgments()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    corpus.validate()?;
    assert_eq!(corpus.cases.len(), VisualQueryClass::all().len());
    let classes = corpus
        .cases
        .iter()
        .map(|case| case.class)
        .collect::<BTreeSet<_>>();
    assert_eq!(classes, VisualQueryClass::all().into_iter().collect());
    assert!(corpus.cases.iter().any(|case| {
        case.judgments
            .iter()
            .any(|judgment| judgment.kind == maestria_retrieval::VisualEvidenceKind::Page)
    }));
    assert!(corpus.cases.iter().any(|case| {
        case.judgments
            .iter()
            .any(|judgment| judgment.kind == maestria_retrieval::VisualEvidenceKind::Region)
    }));
    Ok(())
}

#[test]
fn visual_benchmark_promotes_only_measured_winning_query_classes()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    let comparison = VisualBenchmarkComparison::evaluate(&corpus, &observations(&corpus)?)?;
    let promotion = comparison.promotion("visual-evaluation-v1".to_string())?;
    let expected = [
        VisualQueryClass::Table,
        VisualQueryClass::Chart,
        VisualQueryClass::Figure,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(promotion.winning_classes(), &expected);
    assert_eq!(comparison.corpus_id(), "visual-retrieval-benchmark-v1");
    Ok(())
}

#[test]
fn visual_budget_or_security_regressions_block_promotion() -> Result<(), Box<dyn std::error::Error>>
{
    let corpus = corpus()?;
    let mut measured = observations(&corpus)?;
    for observation in &mut measured {
        if observation.route == VisualRoute::Visual && observation.case_id == "table-001" {
            observation.latency_ms = 1_000;
            observation.security_violations = 1;
        }
    }
    let comparison = VisualBenchmarkComparison::evaluate(&corpus, &measured)?;
    let promotion = comparison.promotion("visual-regressed".to_string())?;
    assert!(
        !promotion
            .winning_classes()
            .contains(&VisualQueryClass::Table)
    );
    Ok(())
}

#[test]
fn visual_execution_policy_is_shadowed_until_class_promotion()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    let comparison = VisualBenchmarkComparison::evaluate(&corpus, &observations(&corpus)?)?;
    let promotion = comparison.promotion("visual-policy-v1".to_string())?;
    let shadow = VisualExecutionPolicy::default();
    assert_eq!(shadow.route_for("find the table"), VisualRoute::TextLayout);
    let active = VisualExecutionPolicy::Active(promotion);
    assert_eq!(active.route_for("find the table"), VisualRoute::Visual);
    assert_eq!(
        active.route_for("find the formula"),
        VisualRoute::TextLayout
    );
    assert_eq!(
        active.route_for("unclassified visual question"),
        VisualRoute::TextLayout
    );
    Ok(())
}

#[test]
fn unavailable_visual_provider_is_explicit_and_cannot_promote()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    let executor = VisualProviderUnavailableExecutor::new(
        corpus.corpus_id.clone(),
        corpus.corpus_revision.clone(),
        "no configured visual provider",
    );
    let observations = run_visual_benchmark(&corpus, &executor)?;
    assert!(observations.iter().all(|observation| {
        !observation.provider_status.is_available() && observation.page_region_recall.value() == 0
    }));
    let comparison = VisualBenchmarkComparison::evaluate(&corpus, &observations)?;
    let promotion = comparison.promotion("visual-unavailable".to_string())?;
    assert!(promotion.winning_classes().is_empty());
    if let Ok(report_dir) = std::env::var("MAESTRIA_BENCHMARK_REPORT_DIR") {
        #[derive(serde::Serialize)]
        struct Report<'a> {
            measurement_kind: &'static str,
            corpus_id: &'a str,
            corpus_revision: &'a str,
            provider_status: &'static str,
            observations: &'a [VisualBenchmarkObservation],
        }
        fs::create_dir_all(&report_dir)?;
        let report = Report {
            measurement_kind: "real_visual_provider_unavailable",
            corpus_id: &corpus.corpus_id,
            corpus_revision: &corpus.corpus_revision,
            provider_status: "unavailable",
            observations: &observations,
        };
        fs::write(
            std::path::Path::new(&report_dir).join("visual-provider-unavailable.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
    }
    Ok(())
}

#[test]
fn visual_text_layout_executor_produces_available_text_and_unavailable_visual()
-> Result<(), Box<dyn std::error::Error>> {
    let corpus = corpus()?;
    let executor =
        VisualTextLayoutExecutor::new(corpus.corpus_id.clone(), corpus.corpus_revision.clone());
    let observations = run_visual_benchmark(&corpus, &executor)?;
    assert_eq!(observations.len(), corpus.cases.len() * 2);
    for obs in &observations {
        assert!(!obs.evaluation_date.is_empty());
        assert!(!obs.model_fingerprint.is_empty());
        match obs.route {
            VisualRoute::TextLayout => {
                assert!(obs.provider_status.is_available());
                assert!(obs.page_region_recall.value() > 0);
                assert!(obs.latency_ms > 0);
            }
            VisualRoute::Visual => {
                assert!(!obs.provider_status.is_available());
                assert_eq!(obs.page_region_recall.value(), 0);
            }
        }
    }
    Ok(())
}

#[test]
fn visual_executor_policy_vs_measurement_distinction() -> Result<(), Box<dyn std::error::Error>> {
    // Verify that measurement provenance (TextLayout vs Visual route) is
    // kept distinct from policy decisions (Shadow vs Active).  A measurement
    // can report Available while the policy still routes through TextLayout
    // — the two axes are orthogonal.
    let corpus = corpus()?;
    let executor =
        VisualTextLayoutExecutor::new(corpus.corpus_id.clone(), corpus.corpus_revision.clone());
    let observations = run_visual_benchmark(&corpus, &executor)?;
    let comparison = VisualBenchmarkComparison::evaluate(&corpus, &observations)?;
    let promotion = comparison.promotion("visual-policy-measurement-v1".to_string())?;

    // TextLayout measurements report Available, but the default policy is Shadow.
    let shadow_policy = maestria_retrieval::VisualExecutionPolicy::default();
    for obs in &observations {
        if obs.route == VisualRoute::TextLayout {
            assert!(obs.provider_status.is_available());
        }
        // Policy is Shadow → always TextLayout
        assert_eq!(
            shadow_policy.route_for(&obs.case_id),
            VisualRoute::TextLayout
        );
    }

    // Even with an Active policy, only promoted classes use Visual routing.
    let active_policy = maestria_retrieval::VisualExecutionPolicy::Active(promotion);
    for case in &corpus.cases {
        let should_use_visual = active_policy.route_for(&case.query) == VisualRoute::Visual;
        // Provider measurement status is independent of policy.
        let text_layout_obs = observations
            .iter()
            .find(|o| o.case_id == case.case_id && o.route == VisualRoute::TextLayout)
            .ok_or("missing text-layout observation")?;
        assert!(text_layout_obs.provider_status.is_available());
        if should_use_visual {
            let visual_obs = observations
                .iter()
                .find(|o| o.case_id == case.case_id && o.route == VisualRoute::Visual)
                .ok_or("missing visual observation")?;
            // Visual route is unavailable because no provider is configured.
            assert!(!visual_obs.provider_status.is_available());
        }
    }
    Ok(())
}
