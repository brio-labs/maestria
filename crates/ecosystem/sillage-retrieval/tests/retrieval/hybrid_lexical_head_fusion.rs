use sillage_domain::{
    ArtifactVersionId, ContentRange, EvidenceCandidate, EvidenceCandidateDto, EvidenceId,
    EvidenceSpan, FreshnessStatus, IndexGenerationId, RetrievalReason, SearchExecution,
    SearchExecutionBudget, SourceLocation, StructureNodeId, TrustLabel,
};
use sillage_ports::SearchQuery;
use sillage_retrieval::types::CandidateBatch;
use sillage_retrieval::{
    FixedKRrf, HybridLexicalHead, HybridPromotionRecord, LearnedSparseQueryClass, RankFusion,
};

use crate::common::fixture_scores;

fn candidate(
    id: u64,
    lexical_score: u32,
    dense_score: u32,
    freshness: FreshnessStatus,
    duplicate_cluster: Option<u64>,
) -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(id),
        artifact_version: ArtifactVersionId::new(id + 100),
        source_span: EvidenceSpan::new(
            Some(StructureNodeId::new(id)),
            SourceLocation::file(format!("source-{id}.md"), 1, 3)?,
            ContentRange::new(0, 12)?,
        )?,
        scores: fixture_scores(lexical_score, dense_score)?,
        trust: TrustLabel::Verified,
        freshness,
        duplicate_cluster: duplicate_cluster.map(sillage_domain::DuplicateClusterId::new),
        reasons: vec![RetrievalReason::ExactMatch],
        coverage_keys: vec![format!("evidence-{id}")],
    })?)
}

fn descriptor(id: &str, modality: &str) -> sillage_retrieval::types::RetrieverDescriptor {
    sillage_retrieval::types::RetrieverDescriptor {
        id: id.to_string(),
        modality: modality.to_string(),
        representation: sillage_domain::RepresentationName::new(format!("{modality}_v1")),
        generation: IndexGenerationId::new(1),
    }
}

fn batch(id: &str, modality: &str, candidates: Vec<EvidenceCandidate>) -> CandidateBatch {
    CandidateBatch::succeeded(
        descriptor(id, modality),
        "fixture query".to_string(),
        candidates,
        Some(IndexGenerationId::new(1)),
        SearchExecution::default(),
    )
}

fn query() -> SearchQuery {
    SearchQuery {
        q: "fixture query".to_string(),
        limit: 10,
        offset: 0,
        execution_budget: SearchExecutionBudget::default(),
    }
}

#[test]
fn lexical_head_is_retained_while_rank_fusion_improves_the_tail()
-> Result<(), Box<dyn std::error::Error>> {
    let head = candidate(1, 100, 0, FreshnessStatus::UpToDate, None)?;
    let tail = candidate(2, 80, 0, FreshnessStatus::UpToDate, None)?;
    let semantic = candidate(3, 0, 90, FreshnessStatus::UpToDate, None)?;
    let batches = vec![
        batch("lexical_primary", "text", vec![head.clone(), tail.clone()]),
        batch("dense_semantic", "dense", vec![tail, semantic]),
    ];

    let rank_only = FixedKRrf::new(60).fuse(&query(), &batches)?;
    assert_eq!(rank_only[0].candidate.evidence_id(), EvidenceId::new(2));

    let output = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &batches,
        None,
    )?;
    assert_eq!(output.protected_lexical_head, Some(EvidenceId::new(1)));
    assert_eq!(output.candidates[0].candidate, head);
    assert_eq!(
        output.candidates[1].candidate.evidence_id(),
        EvidenceId::new(2)
    );
    Ok(())
}

#[test]
fn stale_lexical_result_is_skipped_for_the_next_eligible_lexical_lane()
-> Result<(), Box<dyn std::error::Error>> {
    let stale = candidate(1, 100, 0, FreshnessStatus::Stale, None)?;
    let eligible = candidate(2, 80, 0, FreshnessStatus::UpToDate, None)?;
    let semantic = candidate(3, 0, 90, FreshnessStatus::UpToDate, None)?;
    let batches = vec![
        batch("lexical_stale", "text", vec![stale]),
        batch("lexical_current", "text", vec![eligible.clone()]),
        batch("dense_semantic", "dense", vec![semantic]),
    ];

    let output = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &batches,
        None,
    )?;
    assert_eq!(output.protected_lexical_head, Some(EvidenceId::new(2)));
    assert_eq!(output.candidates[0].candidate, eligible);
    Ok(())
}

#[test]
fn lexical_baseline_head_comes_from_rrf_across_lexical_lanes()
-> Result<(), Box<dyn std::error::Error>> {
    let a_lexical = candidate(30, 100, 0, FreshnessStatus::UpToDate, None)?;
    let b_lexical = candidate(31, 80, 0, FreshnessStatus::UpToDate, None)?;
    let b_second_lane = candidate(31, 80, 0, FreshnessStatus::UpToDate, None)?;
    let a_dense = candidate(30, 0, 100, FreshnessStatus::UpToDate, None)?;
    let batches = vec![
        batch("lexical_primary", "text", vec![a_lexical, b_lexical]),
        batch("lexical_secondary", "text", vec![b_second_lane]),
        batch("dense_semantic", "dense", vec![a_dense]),
    ];

    let lexical_baseline = FixedKRrf::new(60).fuse(&query(), &batches[..2])?;
    assert_eq!(
        lexical_baseline[0].candidate.evidence_id(),
        EvidenceId::new(31)
    );
    let all_lanes = FixedKRrf::new(60).fuse(&query(), &batches)?;
    assert_eq!(all_lanes[0].candidate.evidence_id(), EvidenceId::new(30));

    let output = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &batches,
        None,
    )?;
    assert_eq!(output.protected_lexical_head, Some(EvidenceId::new(31)));
    assert_eq!(
        output.candidates[0].candidate.evidence_id(),
        EvidenceId::new(31)
    );
    Ok(())
}

#[test]
fn scoreless_lexical_baseline_head_is_skipped_for_next_eligible_candidate()
-> Result<(), Box<dyn std::error::Error>> {
    let scoreless = candidate(40, 0, 0, FreshnessStatus::UpToDate, None)?;
    let eligible = candidate(41, 80, 0, FreshnessStatus::UpToDate, None)?;
    let batches = vec![
        batch(
            "lexical_primary",
            "text",
            vec![scoreless.clone(), eligible.clone()],
        ),
        batch("dense_empty", "dense", Vec::new()),
    ];

    let lexical_baseline = FixedKRrf::new(60).fuse(&query(), &batches[..1])?;
    assert_eq!(
        lexical_baseline[0].candidate.evidence_id(),
        scoreless.evidence_id()
    );
    let output = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &batches,
        None,
    )?;
    assert_eq!(output.protected_lexical_head, Some(eligible.evidence_id()));
    assert_eq!(
        output.candidates[0].candidate.evidence_id(),
        eligible.evidence_id()
    );
    assert!(
        output
            .candidates
            .iter()
            .any(|fused| fused.candidate.evidence_id() == scoreless.evidence_id())
    );
    Ok(())
}

#[test]
fn duplicate_cluster_keeps_exact_lexical_head_and_merges_real_lane_scores()
-> Result<(), Box<dyn std::error::Error>> {
    let head = candidate(20, 100, 0, FreshnessStatus::UpToDate, Some(7))?;
    let lower_id_representative = candidate(10, 0, 90, FreshnessStatus::UpToDate, Some(7))?;
    let batches = vec![
        batch("lexical_primary", "text", vec![head.clone()]),
        batch("dense_semantic", "dense", vec![lower_id_representative]),
    ];

    let rank_only = FixedKRrf::new(60).fuse(&query(), &batches)?;
    assert_eq!(rank_only[0].candidate.evidence_id(), EvidenceId::new(10));

    let output = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &batches,
        None,
    )?;
    let fused_head = &output.candidates[0].candidate;
    assert_eq!(fused_head.evidence_id(), head.evidence_id());
    assert_eq!(fused_head.artifact_version(), head.artifact_version());
    assert_eq!(fused_head.source_span(), head.source_span());
    assert_eq!(fused_head.trust(), head.trust());
    assert_eq!(fused_head.freshness(), head.freshness());
    assert_eq!(fused_head.duplicate_cluster(), head.duplicate_cluster());
    assert_eq!(fused_head.reasons(), head.reasons());
    assert_eq!(fused_head.coverage_keys(), head.coverage_keys());
    assert_eq!(
        fused_head
            .scores()
            .lane(&sillage_domain::RetrievalScoreKind::LexicalBm25)
            .map(|lane| lane.raw_score),
        Some(100)
    );
    assert_eq!(
        fused_head
            .scores()
            .lane(&sillage_domain::RetrievalScoreKind::DenseSimilarity)
            .map(|lane| lane.raw_score),
        Some(90)
    );
    Ok(())
}

#[test]
fn empty_or_failed_lexical_batches_and_shadow_keep_normal_fusion()
-> Result<(), Box<dyn std::error::Error>> {
    let semantic = candidate(3, 0, 90, FreshnessStatus::UpToDate, None)?;
    let failed_lexical = CandidateBatch::failed(
        descriptor("lexical_failed", "text"),
        "fixture query".to_string(),
        "unavailable".to_string(),
        Some(IndexGenerationId::new(1)),
        SearchExecution::default(),
    );
    let hybrid_batches = vec![
        failed_lexical,
        batch("lexical_empty", "text", Vec::new()),
        batch("dense_semantic", "dense", vec![semantic]),
    ];
    let normal = FixedKRrf::new(60).fuse(&query(), &hybrid_batches)?;
    let unanchored = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &hybrid_batches,
        None,
    )?;
    assert_eq!(unanchored.protected_lexical_head, None);
    assert_eq!(unanchored.candidates, normal);

    let head = candidate(1, 100, 0, FreshnessStatus::UpToDate, None)?;
    let tail = candidate(2, 80, 0, FreshnessStatus::UpToDate, None)?;
    let shadow_batches = vec![batch("lexical_primary", "text", vec![head, tail])];
    let normal = FixedKRrf::new(60).fuse(&query(), &shadow_batches)?;
    let shadow = HybridLexicalHead::new(FixedKRrf::new(60)).fuse_with_protected_head(
        &query(),
        &shadow_batches,
        None,
    )?;
    assert_eq!(shadow.protected_lexical_head, None);
    assert_eq!(shadow.candidates, normal);
    Ok(())
}

#[test]
fn old_or_unknown_hybrid_ranking_policy_records_are_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let record = HybridPromotionRecord::new(
        "evaluation".to_string(),
        "2026-10-02".to_string(),
        std::collections::BTreeSet::from([LearnedSparseQueryClass::DomainTerminology]),
        sillage_retrieval::HYBRID_SERVING_POLICY_ID,
    )
    .ok_or("hybrid promotion record was rejected")?;
    assert_eq!(
        record.ranking_policy_id(),
        sillage_retrieval::HYBRID_SERVING_POLICY_ID
    );
    assert_eq!(
        HybridLexicalHead::new(FixedKRrf::new(60)).trace_identity(),
        "hybrid-lexical-head-preserving-v1+fixed-k-rrf-v1:k=60"
    );
    let serialized = serde_json::to_value(&record)?;
    assert_eq!(
        serialized.get("ranking_policy"),
        Some(&serde_json::Value::String(
            sillage_retrieval::HYBRID_SERVING_POLICY_ID.to_string()
        ))
    );

    let mut old_record = serialized.clone();
    old_record
        .as_object_mut()
        .ok_or("serialized promotion record was not an object")?
        .remove("ranking_policy");
    assert!(serde_json::from_value::<HybridPromotionRecord>(old_record).is_err());

    let mut unsupported_record = serialized;
    unsupported_record["ranking_policy"] =
        serde_json::Value::String("rank-only-rrf-v1".to_string());
    assert!(serde_json::from_value::<HybridPromotionRecord>(unsupported_record).is_err());
    Ok(())
}
