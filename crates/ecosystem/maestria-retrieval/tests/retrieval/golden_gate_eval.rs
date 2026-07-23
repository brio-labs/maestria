use maestria_domain::{EvidenceId, SearchStatus};
use maestria_retrieval::golden::Metric;

use crate::common::golden::{candidate, corpus, observation, permissive_gate, plan};

// ── GoldenGate metric computation and validation ────────────────────────────

#[test]
fn golden_gate_reports_relevance_and_exact_span_metrics() -> Result<(), Box<dyn std::error::Error>>
{
    let plan = plan()?;
    let first = candidate(1, 3)?;
    let second = candidate(2, 4)?;
    let corpus = corpus(
        &plan,
        vec![
            maestria_retrieval::golden::GoldenJudgment {
                evidence_id: first.evidence_id,
                relevance: 3,
                exact_span: Some(first.source_span.clone()),
            },
            maestria_retrieval::golden::GoldenJudgment {
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
        vec![maestria_retrieval::golden::GoldenJudgment {
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
        vec![maestria_retrieval::golden::GoldenJudgment {
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
        vec![maestria_retrieval::golden::GoldenJudgment {
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
        vec![maestria_retrieval::golden::GoldenJudgment {
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
        vec![maestria_retrieval::golden::GoldenJudgment {
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
