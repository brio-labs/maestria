use std::collections::BTreeMap;

use maestria_domain::{
    ArtifactVersionId, ContentRange, EvidenceCandidate, EvidenceId, EvidenceSpan, FreshnessStatus,
    LearnedSparseContribution, LearnedSparseReason, RepresentationName, RetrievalLaneScore,
    RetrievalModelFingerprint, RetrievalRawRank, RetrievalReason, RetrievalScoreFingerprint,
    RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet, SourceLocation, TrustLabel,
};

fn fingerprint(name: &str) -> Result<RetrievalScoreFingerprint, Box<dyn std::error::Error>> {
    Ok(RetrievalScoreFingerprint::new(
        RetrievalModelFingerprint::new(name.to_string())?,
        BTreeMap::from([("fixture".to_string(), "retrieval_scores".to_string())]),
    ))
}

fn lane(
    kind: RetrievalScoreKind,
    raw_score: i64,
    raw_rank: RetrievalRawRank,
    representation: &str,
) -> Result<RetrievalLaneScore, Box<dyn std::error::Error>> {
    Ok(RetrievalLaneScore::new(
        kind,
        raw_score,
        raw_rank,
        RetrievalScoreScale::unbounded("fixture_raw"),
        RepresentationName::new(representation),
        fingerprint(&format!("fixture:{representation}"))?,
    ))
}

#[test]
fn current_score_schema_round_trips_canonically() -> Result<(), Box<dyn std::error::Error>> {
    let scores = RetrievalScoreSet::new(vec![
        lane(
            RetrievalScoreKind::LearnedSparse,
            40,
            RetrievalRawRank::ranked(2),
            "sparse_text_v1",
        )?,
        lane(
            RetrievalScoreKind::LexicalBm25,
            80,
            RetrievalRawRank::ranked(1),
            "lexical_text_v1",
        )?,
    ])?;
    let encoded = serde_json::to_string(&scores)?;
    let decoded: RetrievalScoreSet = serde_json::from_str(&encoded)?;
    assert_eq!(decoded, scores);
    assert!(encoded.contains("schema_version"));
    assert!(!encoded.contains("\"bm25\""));
    assert!(!encoded.contains("semantic_similarity"));
    Ok(())
}

#[test]
fn legacy_fixed_scores_migrate_once_without_zero_lanes() -> Result<(), Box<dyn std::error::Error>> {
    let migrated: RetrievalScoreSet =
        serde_json::from_str(r#"{"bm25":91,"semantic_similarity":0}"#)?;
    assert_eq!(migrated.lanes.len(), 1);
    let lexical = migrated
        .lane(&RetrievalScoreKind::LexicalBm25)
        .ok_or("missing migrated lexical lane")?;
    assert_eq!(lexical.raw_score, 91);
    assert!(matches!(
        lexical.raw_rank,
        RetrievalRawRank::Unavailable { .. }
    ));
    let encoded = serde_json::to_string(&migrated)?;
    assert!(!encoded.contains("\"bm25\""));
    assert!(!encoded.contains("semantic_similarity"));
    Ok(())
}

#[test]
fn legacy_sparse_reason_moves_score_into_the_canonical_lane()
-> Result<(), Box<dyn std::error::Error>> {
    let mut value = serde_json::to_value(EvidenceCandidate {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(2),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: "fixture.md".to_string(),
                start_line: 1,
                end_line: 1,
            },
            ContentRange { start: 1, end: 1 },
        )?,
        scores: RetrievalScoreSet::empty(),
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::LearnedSparse(Box::new(
            LearnedSparseReason::new(vec![LearnedSparseContribution {
                term_id: 7,
                contribution_micros: 12,
            }]),
        ))],
        coverage_keys: Vec::new(),
    })?;
    value["scores"] = serde_json::json!({"bm25": 0, "semantic_similarity": 0});
    value["reasons"] = serde_json::json!([{
        "LearnedSparse": {
            "score_micros": 12,
            "representation": "sparse_text_v1",
            "fingerprint": "fixture:sparse:v1",
            "contributions": [{"term_id": 7, "contribution_micros": 12}]
        }
    }]);
    let migrated: EvidenceCandidate = serde_json::from_value(value)?;
    let sparse = migrated
        .scores
        .lane(&RetrievalScoreKind::LearnedSparse)
        .ok_or("missing migrated sparse lane")?;
    assert_eq!(sparse.raw_score, 12);
    assert!(matches!(
        sparse.raw_rank,
        RetrievalRawRank::Unavailable { .. }
    ));
    let encoded = serde_json::to_value(&migrated)?;
    assert!(!encoded["reasons"].to_string().contains("score_micros"));
    Ok(())
}

#[test]
fn malformed_or_duplicate_score_provenance_fails_closed() -> Result<(), Box<dyn std::error::Error>>
{
    let duplicate = RetrievalScoreSet::new(vec![
        lane(
            RetrievalScoreKind::Graph,
            10,
            RetrievalRawRank::ranked(1),
            "graph_context_v1",
        )?,
        lane(
            RetrievalScoreKind::Graph,
            9,
            RetrievalRawRank::ranked(2),
            "graph_context_v1",
        )?,
    ]);
    assert!(duplicate.is_err());

    let invalid_rank = RetrievalScoreSet::single(lane(
        RetrievalScoreKind::Exact,
        1,
        RetrievalRawRank::ranked(0),
        "exact_v1",
    )?);
    assert!(invalid_rank.is_err());

    let unsupported =
        serde_json::from_str::<RetrievalScoreSet>(r#"{"schema_version":999,"lanes":[]}"#);
    assert!(unsupported.is_err());
    Ok(())
}

#[test]
fn every_declared_score_kind_has_one_canonical_wire_shape() -> Result<(), Box<dyn std::error::Error>>
{
    let kinds = vec![
        RetrievalScoreKind::Exact,
        RetrievalScoreKind::LexicalBm25,
        RetrievalScoreKind::DenseSimilarity,
        RetrievalScoreKind::LearnedSparse,
        RetrievalScoreKind::LateInteraction,
        RetrievalScoreKind::Graph,
        RetrievalScoreKind::SpecializedRetrieval {
            route: "repository_code".to_string(),
        },
    ];
    let lanes = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            lane(
                kind,
                i64::try_from(index.saturating_add(1)).unwrap_or(i64::MAX),
                RetrievalRawRank::ranked(
                    u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX),
                ),
                &format!("fixture_representation_{index}"),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let scores = RetrievalScoreSet::new(lanes)?;
    assert_eq!(scores.lanes.len(), 7);
    let json = serde_json::to_string(&scores)?;
    assert!(json.contains("late_interaction"));
    assert!(json.contains("graph"));
    assert!(json.contains("specialized_retrieval"));
    assert_eq!(serde_json::from_str::<RetrievalScoreSet>(&json)?, scores);
    Ok(())
}

#[test]
fn complete_fingerprint_and_rank_change_the_trace_identity()
-> Result<(), Box<dyn std::error::Error>> {
    use maestria_domain::{
        CorpusScope, CorpusSnapshotId, EvidenceRequirements, FreshnessRequirement,
        IndexGenerationId, Modality, ModalitySet, QueryId, SearchBudget, SearchIntent, SearchPlan,
        SearchStage, SearchStopReason, SearchTrace, StopConditions,
    };

    let plan = SearchPlan {
        query_id: QueryId::new(1),
        original_query: "trace provenance".to_string(),
        intent: SearchIntent::FactualLocal,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(2),
        index_generation: IndexGenerationId::new(3),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text]),
        stages: vec![SearchStage::InitialRetrieval],
        budgets: SearchBudget::new(64, 1_000)?,
        stop_conditions: StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        },
        fingerprint: RetrievalModelFingerprint::new("trace-model-v1".to_string())?,
        original_intent: None,
        route_decision: None,
    };
    let mut candidate = EvidenceCandidate {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(2),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: "fixture.md".to_string(),
                start_line: 1,
                end_line: 1,
            },
            ContentRange { start: 1, end: 1 },
        )?,
        scores: RetrievalScoreSet::single(lane(
            RetrievalScoreKind::DenseSimilarity,
            900_000,
            RetrievalRawRank::ranked(1),
            "dense_text_v1",
        )?)?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::SemanticSimilarity],
        coverage_keys: Vec::new(),
    };
    let first = SearchTrace::from_plan(
        &plan,
        vec!["dense".to_string()],
        std::slice::from_ref(&candidate),
        Vec::new(),
        None,
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    )
    .deterministic_id();

    candidate.scores.lanes[0]
        .fingerprint
        .components
        .insert("revision".to_string(), "v2".to_string());
    let second = SearchTrace::from_plan(
        &plan,
        vec!["dense".to_string()],
        std::slice::from_ref(&candidate),
        Vec::new(),
        None,
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    )
    .deterministic_id();
    assert_ne!(first, second);

    candidate.scores.lanes[0].raw_rank = RetrievalRawRank::ranked(2);
    let third = SearchTrace::from_plan(
        &plan,
        vec!["dense".to_string()],
        std::slice::from_ref(&candidate),
        Vec::new(),
        None,
        Vec::new(),
        SearchStopReason::EvidenceComplete,
    )
    .deterministic_id();
    assert_ne!(second, third);
    Ok(())
}
