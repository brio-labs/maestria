use std::collections::BTreeMap;

use maestria_domain::{
    ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, EvidenceCandidate,
    EvidenceCandidateDto, EvidenceId, EvidenceRequirements, EvidenceSpan, FreshnessRequirement,
    FreshnessStatus, IndexGenerationId, LearnedSparseContribution, LearnedSparseReason, Modality,
    ModalitySet, QueryId, RepresentationName, RetrievalLaneScore, RetrievalModelFingerprint,
    RetrievalRawRank, RetrievalReason, RetrievalScoreFingerprint, RetrievalScoreKind,
    RetrievalScoreScale, RetrievalScoreSet, SearchBudget, SearchIntent, SearchPlan, SearchStage,
    SearchStopReason, SearchTrace, SearchTraceId, SourceLocation, StopConditions, TrustLabel,
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
fn current_score_schema_decodes_from_json() -> Result<(), Box<dyn std::error::Error>> {
    let json = r#"{"schema_version":2,"lanes":[{"score_kind":"lexical_bm25","raw_score":80,"raw_rank":{"state":"ranked","rank":1},"scale":{"kind":"unbounded","name":"fixture_raw","higher_is_better":true},"representation":"lexical_text_v1","fingerprint":{"identity":"fixture:lexical_text_v1","components":{"fixture":"retrieval_scores"}}}]}"#;
    let decoded: RetrievalScoreSet = serde_json::from_str(json)?;
    assert_eq!(decoded.schema_version, 2);
    assert_eq!(decoded.lanes.len(), 1);
    let lexical = decoded
        .lane(&RetrievalScoreKind::LexicalBm25)
        .ok_or("missing decoded lexical lane")?;
    assert_eq!(lexical.raw_score, 80);
    let encoded = serde_json::to_string(&decoded)?;
    assert!(!encoded.contains("\"bm25\""));
    assert!(!encoded.contains("semantic_similarity"));
    Ok(())
}

#[test]
fn legacy_flat_score_payload_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let legacy =
        serde_json::from_str::<RetrievalScoreSet>(r#"{"bm25":91,"semantic_similarity":0}"#);
    assert!(
        legacy.is_err(),
        "legacy flat score payload must fail strict decode"
    );
    Ok(())
}

#[test]
fn learned_sparse_reason_round_trips_current_shape() -> Result<(), Box<dyn std::error::Error>> {
    let candidate = EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(2),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::file("fixture.md".to_string(), 1, 1)?,
            ContentRange::new(1, 1)?,
        )?,
        scores: RetrievalScoreSet::single(lane(
            RetrievalScoreKind::LearnedSparse,
            40,
            RetrievalRawRank::ranked(2),
            "sparse_text_v1",
        )?)?,
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
    let value = serde_json::to_value(&candidate)?;
    let decoded: EvidenceCandidate = serde_json::from_value(value.clone())?;
    assert_eq!(decoded, candidate);
    let encoded = serde_json::to_value(&decoded)?;
    assert!(!encoded["reasons"].to_string().contains("score_micros"));
    Ok(())
}

#[test]
fn legacy_sparse_reason_payload_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let mut value = serde_json::to_value(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(2),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::file("fixture.md".to_string(), 1, 1)?,
            ContentRange::new(1, 1)?,
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
    })?)?;
    value["reasons"] = serde_json::from_str(
        r#"[{"LearnedSparse":{"score_micros":12,"representation":"sparse_text_v1","fingerprint":"fixture:sparse:v1","contributions":[{"term_id":7,"contribution_micros":12}]}}]"#,
    )?;
    assert!(
        serde_json::from_value::<EvidenceCandidate>(value).is_err(),
        "legacy score-bearing reason payload must fail strict decode"
    );
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
                match i64::try_from(index.saturating_add(1)) {
                    Ok(value) => value,
                    Err(e) => {
                        let _ = e;
                        i64::MAX
                    }
                },
                RetrievalRawRank::ranked(match u32::try_from(index.saturating_add(1)) {
                    Ok(value) => value,
                    Err(e) => {
                        let _ = e;
                        u32::MAX
                    }
                }),
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
    let plan = provenance_plan()?;
    let mut candidate = EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(1),
        artifact_version: ArtifactVersionId::new(2),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::file("fixture.md".to_string(), 1, 1)?,
            ContentRange::new(1, 1)?,
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
    })?;
    let trace_for =
        |candidate: &EvidenceCandidate| -> Result<SearchTraceId, Box<dyn std::error::Error>> {
            Ok(SearchTrace::from_plan(
                &plan,
                vec!["dense".to_string()],
                std::slice::from_ref(candidate),
                Vec::new(),
                None,
                Vec::new(),
                SearchStopReason::EvidenceComplete,
            )?
            .deterministic_id())
        };
    let first = trace_for(&candidate)?;

    let mut revised_scores = candidate.scores().clone();
    revised_scores.lanes[0]
        .fingerprint
        .components
        .insert("revision".to_string(), "v2".to_string());
    candidate = EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: candidate.evidence_id(),
        artifact_version: candidate.artifact_version(),
        source_span: candidate.source_span().clone(),
        scores: revised_scores,
        trust: candidate.trust(),
        freshness: candidate.freshness(),
        duplicate_cluster: candidate.duplicate_cluster(),
        reasons: candidate.reasons().to_vec(),
        coverage_keys: candidate.coverage_keys().to_vec(),
    })?;
    let second = trace_for(&candidate)?;
    assert_ne!(first, second);

    let mut revised_scores = candidate.scores().clone();
    revised_scores.lanes[0].raw_rank = RetrievalRawRank::ranked(2);
    candidate = EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: candidate.evidence_id(),
        artifact_version: candidate.artifact_version(),
        source_span: candidate.source_span().clone(),
        scores: revised_scores,
        trust: candidate.trust(),
        freshness: candidate.freshness(),
        duplicate_cluster: candidate.duplicate_cluster(),
        reasons: candidate.reasons().to_vec(),
        coverage_keys: candidate.coverage_keys().to_vec(),
    })?;
    let third = trace_for(&candidate)?;
    assert_ne!(second, third);
    Ok(())
}

/// Shared plan for the trace-identity provenance tests.
fn provenance_plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query("trace provenance".to_string())
        .intent(SearchIntent::FactualLocal)
        .scope(CorpusScope::Global)
        .corpus_snapshot(CorpusSnapshotId::new(2))
        .index_generation(IndexGenerationId::new(3))
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Text]))
        .stages(vec![SearchStage::InitialRetrieval])
        .budgets(SearchBudget::new(64, 1_000)?)
        .stop_conditions(StopConditions {
            max_results: 5,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
            require_primary_sources: false,
            minimum_corroboration: 1,
        })
        .fingerprint(RetrievalModelFingerprint::new(
            "trace-model-v1".to_string(),
        )?)
        .authorization(Some(
            maestria_domain::RetrievalPolicySnapshot::global_default(),
        ))
        .build()?)
}
