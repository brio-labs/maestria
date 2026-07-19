use std::collections::BTreeSet;

use maestria_retrieval::golden::Metric;
use maestria_retrieval::{
    VisualBenchmarkCase, VisualBenchmarkComparison, VisualBenchmarkCorpus, VisualBenchmarkError,
    VisualBenchmarkObservation, VisualExecutionPolicy, VisualQueryClass, VisualRoute,
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
    Ok(run_visual_benchmark(corpus, &move |case, route| {
        let profile = profile(&case, route)
            .map_err(|error| VisualBenchmarkError::InvalidCorpus(error.to_string()))?;
        Ok(VisualBenchmarkObservation {
            corpus_id: corpus_id.clone(),
            corpus_revision: corpus_revision.clone(),
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
