use std::collections::BTreeSet;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, Evidence,
    EvidenceCandidate, EvidenceCandidateDto, EvidenceId, EvidenceKind, EvidenceRequirements,
    EvidenceSpan, FreshnessRequirement, FreshnessStatus, IndexFingerprint, IndexGenerationId,
    IndexStatus, LineRange, LogicalTick, Modality, ModalitySet, QueryId, RetrievalLaneScore,
    RetrievalModelFingerprint, RetrievalRawRank, RetrievalReason, RetrievalScoreFingerprint,
    RetrievalScoreKind, RetrievalScoreScale, RetrievalScoreSet, SearchBudget, SearchIntent,
    SearchPlan, SearchStage, SecurityMetadata, SnapshotRef, SourceLocation, StopConditions,
    TrustLabel,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, EvidenceRepository, InMemoryArtifactRepository,
    InMemoryBlobStore, InMemoryEvidenceRepository, LateInteractionProvider, MULTIVECTOR_TEXT_V1,
    MultiVectorCompression, MultiVectorDocument, MultiVectorDocumentRequest,
    MultiVectorFingerprint, MultiVectorIdentity, MultiVectorQuery, MultiVectorQueryRequest,
    MultiVectorSimilarity, MultiVectorToken, MultiVectorTokenPolicy, PortError,
    ProviderCallControl, ProviderDisclosure, RetentionPolicy,
};

use super::{LateInteractionReranker, LateInteractionRerankerParts};
use crate::traits::CandidateReranker;
use crate::types::{CandidateSourceFilter, RankedCandidate, RerankLimits, RerankRequest};
use crate::{MaxSimLateInteractionScorer, SearchCancellation};

struct FakeLateProvider {
    identity: MultiVectorIdentity,
    query_calls: AtomicUsize,
    document_calls: AtomicUsize,
    document_texts: Mutex<Vec<String>>,
    fail_document_at: Option<usize>,
    negative: bool,
}

impl FakeLateProvider {
    fn new(identity: MultiVectorIdentity, fail_document_at: Option<usize>, negative: bool) -> Self {
        Self {
            identity,
            query_calls: AtomicUsize::new(0),
            document_calls: AtomicUsize::new(0),
            document_texts: Mutex::new(Vec::new()),
            fail_document_at,
            negative,
        }
    }

    fn query_calls(&self) -> usize {
        self.query_calls.load(Ordering::Acquire)
    }

    fn document_calls(&self) -> usize {
        self.document_calls.load(Ordering::Acquire)
    }

    fn document_texts(&self) -> Option<Vec<String>> {
        self.document_texts.lock().ok().map(|texts| texts.clone())
    }
}

impl LateInteractionProvider for FakeLateProvider {
    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        })
    }

    fn identity(&self) -> Option<MultiVectorIdentity> {
        Some(self.identity.clone())
    }

    fn encode_query(
        &self,
        _text: &str,
        request: &MultiVectorQueryRequest,
        _control: &dyn ProviderCallControl,
    ) -> Result<MultiVectorQuery, PortError> {
        request.validate()?;
        self.query_calls.fetch_add(1, Ordering::AcqRel);
        MultiVectorQuery::new(
            self.identity.clone(),
            request.input_hash.clone(),
            1,
            false,
            vec![MultiVectorToken::new(0, vec![1.0, 0.0])?],
        )
    }

    fn encode_document(
        &self,
        text: &str,
        request: &MultiVectorDocumentRequest,
        _control: &dyn ProviderCallControl,
    ) -> Result<MultiVectorDocument, PortError> {
        request.validate()?;
        self.document_texts
            .lock()
            .map_err(|_| PortError::internal("fake late provider", "text lock poisoned"))?
            .push(text.to_owned());
        let call = self.document_calls.fetch_add(1, Ordering::AcqRel) + 1;
        if self.fail_document_at == Some(call) {
            return Err(PortError::downstream(
                "fake late provider",
                "document failure",
            ));
        }
        let value = if self.negative {
            -1.0
        } else {
            match request.source.evidence_id.value() {
                1 => 0.1,
                2 => 0.5,
                3 => 1.0,
                _ => 0.25,
            }
        };
        MultiVectorDocument::new(
            self.identity.clone(),
            request.source.clone(),
            1,
            false,
            vec![MultiVectorToken::new(0, vec![value, 0.0])?],
        )
    }
}

fn identity() -> Result<MultiVectorIdentity, Box<dyn std::error::Error>> {
    let hash = maestria_domain::ContentHash::new(format!("sha256:{}", "a".repeat(64)))?;
    let policy = |prefix_id: u32, max_vectors, max_utf8_bytes| MultiVectorTokenPolicy {
        prefix_id: prefix_id.into(),
        prefix_text: "prefix".into(),
        native_token_limit: 8192,
        max_vectors,
        max_utf8_bytes,
        truncate_right_after_prefix: true,
        retain_special_tokens: true,
        drop_masked_positions: true,
        pad_token_id: 0.into(),
        query_expansion: false,
        lowercase: false,
        punctuation_pruning: false,
    };
    Ok(MultiVectorIdentity {
        representation: MULTIVECTOR_TEXT_V1.into(),
        fingerprint: MultiVectorFingerprint {
            base: IndexFingerprint {
                provider: "mlateon-onnx".into(),
                model: "mlateon".into(),
                revision: "local".into(),
                artifact_hash: hash.clone(),
                dimensions: 2,
                quantization: "onnx_int8".into(),
                query_template_hash: hash.clone(),
                document_template_hash: hash.clone(),
                preprocessing_version: "test".into(),
            },
            tokenizer_hash: hash.clone(),
            vocabulary_hash: hash,
            vocabulary_size: 4,
            similarity: MultiVectorSimilarity::Cosine,
            aggregation: maestria_domain::LateInteractionAggregation::QueryTokenMaxThenSum,
            normalization_version: "L2PerTokenV1".into(),
            representation_compression: MultiVectorCompression::None,
            query_policy: policy(1, 128, 8192),
            document_policy: policy(2, 512, 65_536),
        },
        generation_id: IndexGenerationId::new(1),
        corpus_snapshot: CorpusSnapshotId::new(1),
        realm: maestria_domain::RealmId::try_from("b".repeat(64))?,
        trust_zone: maestria_domain::TrustZone::Verified,
    })
}

fn score_set() -> Result<RetrievalScoreSet, maestria_domain::SearchCompatibilityError> {
    let representation = maestria_domain::RepresentationName::new("lexical_text_v1");
    RetrievalScoreSet::single(RetrievalLaneScore::new(
        RetrievalScoreKind::LexicalBm25,
        1,
        RetrievalRawRank::ranked(1),
        RetrievalScoreScale::unbounded("fixture"),
        representation.clone(),
        RetrievalScoreFingerprint::new(
            RetrievalModelFingerprint::new("fixture:lexical".into())?,
            [("representation".into(), representation.as_str().into())]
                .into_iter()
                .collect(),
        ),
    ))
}

fn candidate(
    id: u64,
    reason: RetrievalReason,
) -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    candidate_at_path(id, reason, format!("fixture-{id}.txt"))
}

fn candidate_at_path(
    id: u64,
    reason: RetrievalReason,
    path: String,
) -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate::new(EvidenceCandidateDto {
        evidence_id: EvidenceId::new(id),
        artifact_version: ArtifactVersionId::new(1),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::file(path, id as u32, id as u32)?,
            ContentRange::new(0, 1)?,
        )?,
        scores: score_set()?,
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![reason],
        coverage_keys: Vec::new(),
    })?)
}

fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan::builder()
        .query_id(QueryId::new(1))
        .original_query("find governed reranking".into())
        .intent(SearchIntent::FactualLocal)
        .scope(CorpusScope::Global)
        .corpus_snapshot(CorpusSnapshotId::new(1))
        .index_generation(IndexGenerationId::new(1))
        .freshness(FreshnessRequirement::Any)
        .modalities(ModalitySet::new(vec![Modality::Text]))
        .stages(vec![SearchStage::InitialRetrieval, SearchStage::Reranking])
        .budgets(SearchBudget::with_resource_limits(100, 100, 1, 2, 0, 0, 1)?)
        .stop_conditions(StopConditions {
            max_results: 10,
            min_score_threshold: 0,
        })
        .evidence_requirements(EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        })
        .fingerprint(RetrievalModelFingerprint::new("fixture:plan".into())?)
        .authorization(maestria_domain::RetrievalPolicySnapshot::global_default())
        .build()?)
}

type FixtureResult = Result<
    (
        LateInteractionReranker,
        Arc<FakeLateProvider>,
        Vec<RankedCandidate>,
    ),
    Box<dyn std::error::Error>,
>;

fn fixture(
    limits: RerankLimits,
    fail_document_at: Option<usize>,
    negative: bool,
    protected: bool,
) -> FixtureResult {
    let identity = identity()?;
    let provider = Arc::new(FakeLateProvider::new(
        identity.clone(),
        fail_document_at,
        negative,
    ));
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    let blobs = Arc::new(InMemoryBlobStore::new());
    let bytes = b"alpha\nbeta\ngamma\ndelta\npassword=super-secret-value\n".to_vec();
    let hash = maestria_domain::ContentHash::new(maestria_domain::content_hash(&bytes))?;
    let blob = blobs.put(bytes)?;
    artifacts.put(Artifact {
        id: ArtifactId::new(1),
        title: "late fixture".into(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: (1..=4).map(EvidenceId::new).collect(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(hash.clone()),
        parse_status: None,
        security: SecurityMetadata::default(),
    })?;
    let reason = if protected {
        RetrievalReason::ExactMatch
    } else {
        RetrievalReason::SemanticSimilarity
    };
    let mut candidates = Vec::new();
    for id in 1..=4 {
        let excerpt = match id {
            1 => "alpha",
            2 => "beta",
            3 => "gamma",
            _ => "delta",
        };
        evidence.put(Evidence {
            id: EvidenceId::new(id),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: format!("fixture-{id}.txt"),
                range: LineRange::new(id as usize, id as usize)?,
                snapshot: SnapshotRef::new(blob, hash.clone()),
            },
            excerpt: excerpt.into(),
            observed_at: LogicalTick::new(1),
            security: SecurityMetadata::default(),
        })?;
        candidates.push(RankedCandidate {
            candidate: candidate(id, reason.clone())?,
            rank: id as usize + 3,
        });
    }
    let reranker = LateInteractionReranker::new(
        LateInteractionRerankerParts {
            artifacts,
            evidence,
            blobs,
            provider: provider.clone(),
            scorer: Arc::new(MaxSimLateInteractionScorer::new()),
            identity,
        },
        limits,
    )?;
    Ok((reranker, provider, candidates))
}

fn request(
    plan: SearchPlan,
    candidates: Vec<RankedCandidate>,
    cancellation: Arc<SearchCancellation>,
    source_filter: Option<CandidateSourceFilter>,
) -> Result<RerankRequest, Box<dyn std::error::Error>> {
    let authorization = Arc::new(
        RetrievalSecurityPolicy::default()
            .authorization_context(plan.scope())
            .map_err(|error| format!("authorization context: {error}"))?,
    );
    Ok(RerankRequest {
        plan: Arc::new(plan),
        candidates,
        max_latency_ms: 250,
        authorization,
        source_filter,
        cancellation,
    })
}

#[test]
fn provider_failure_after_partial_work_preserves_exact_baseline()
-> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, candidates) = fixture(
        RerankLimits {
            input_cap: 4,
            score_cap: 4,
            output_cap: 4,
        },
        Some(2),
        false,
        false,
    )?;
    let baseline = candidates.clone();
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    assert_eq!(provider.query_calls(), 1);
    assert_eq!(provider.document_calls(), 2);
    assert_eq!(result.candidates, baseline);
    assert!(result.trace.candidates.iter().all(|trace| matches!(
        trace.position,
        maestria_domain::RerankPosition::ErrorFallback(ref code)
            if code == "late_interaction:provider_unavailable"
    )));
    Ok(())
}

#[test]
fn unequal_caps_reorder_only_scored_slots_without_duplicates()
-> Result<(), Box<dyn std::error::Error>> {
    let (reranker, _provider, candidates) = fixture(
        RerankLimits {
            input_cap: 4,
            score_cap: 3,
            output_cap: 1,
        },
        None,
        false,
        false,
    )?;
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    let ids: Vec<u64> = result
        .candidates
        .iter()
        .map(|candidate| candidate.candidate.evidence_id().value())
        .collect();
    assert_eq!(ids, vec![3, 1, 2, 4]);
    assert_eq!(
        result
            .candidates
            .iter()
            .map(|candidate| candidate.rank)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert!(matches!(
        result.trace.candidates[0].position,
        maestria_domain::RerankPosition::SkippedCap
    ));
    assert!(result.trace.candidates[2].late_interaction.is_some());
    assert!(result.trace.candidates[3].late_interaction.is_none());
    Ok(())
}

#[test]
fn protected_candidates_do_not_call_provider() -> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, candidates) = fixture(
        RerankLimits {
            input_cap: 4,
            score_cap: 4,
            output_cap: 4,
        },
        None,
        false,
        true,
    )?;
    let baseline = candidates.clone();
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    assert_eq!(provider.query_calls(), 0);
    assert_eq!(provider.document_calls(), 0);
    assert_eq!(result.candidates, baseline);
    Ok(())
}

#[test]
fn source_filter_narrowing_fails_closed_before_document_encoding()
-> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, candidates) = fixture(
        RerankLimits {
            input_cap: 4,
            score_cap: 4,
            output_cap: 4,
        },
        None,
        false,
        false,
    )?;
    let baseline = candidates.clone();
    let filter = CandidateSourceFilter::try_new([ArtifactId::new(2)].into_iter().collect())?;
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        Some(filter),
    )?)?;
    assert_eq!(provider.query_calls(), 1);
    assert_eq!(provider.document_calls(), 0);
    assert_eq!(result.candidates, baseline);
    assert!(matches!(
        result.trace.candidates[0].position,
        maestria_domain::RerankPosition::ErrorFallback(ref code)
            if code == "late_interaction:privacy_rejected"
    ));
    Ok(())
}

#[test]
fn cancellation_before_query_prevents_provider_calls() -> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, candidates) = fixture(
        RerankLimits {
            input_cap: 4,
            score_cap: 4,
            output_cap: 4,
        },
        None,
        false,
        false,
    )?;
    let baseline = candidates.clone();
    let cancellation = Arc::new(SearchCancellation::new());
    cancellation.cancel();
    let result = reranker.rerank(request(plan()?, candidates, cancellation, None)?)?;
    assert_eq!(provider.query_calls(), 0);
    assert_eq!(provider.document_calls(), 0);
    assert_eq!(result.candidates, baseline);
    assert!(matches!(
        result.trace.candidates[0].position,
        maestria_domain::RerankPosition::ErrorFallback(ref code)
            if code == "late_interaction:cancelled"
    ));
    Ok(())
}

#[test]
fn negative_maxsim_and_provenance_retain_original_rank() -> Result<(), Box<dyn std::error::Error>> {
    let (reranker, _provider, mut candidates) = fixture(
        RerankLimits {
            input_cap: 1,
            score_cap: 1,
            output_cap: 1,
        },
        None,
        true,
        false,
    )?;
    candidates[0].rank = 7;
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    let provenance = result.trace.candidates[0]
        .late_interaction
        .as_ref()
        .ok_or("late provenance")?;
    assert_eq!(provenance.score.raw_score, -1_000_000);
    assert_eq!(
        provenance.score.raw_rank,
        RetrievalRawRank::Ranked { rank: 8 }
    );
    assert!(provenance.validate().is_ok());
    Ok(())
}
#[test]
fn malformed_late_provenance_json_fails_validation() -> Result<(), Box<dyn std::error::Error>> {
    let (reranker, _provider, candidates) = fixture(
        RerankLimits {
            input_cap: 1,
            score_cap: 1,
            output_cap: 1,
        },
        None,
        false,
        false,
    )?;
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    let provenance = result.trace.candidates[0]
        .late_interaction
        .as_ref()
        .ok_or("late provenance")?;
    let mut value = serde_json::to_value(provenance)?;
    value["contributions"] = serde_json::Value::Array(Vec::new());
    value["omitted_contribution_count"] = serde_json::Value::from(0_u32);
    let parsed: maestria_domain::LateInteractionProvenance = serde_json::from_value(value)?;
    assert!(parsed.validate().is_err());
    Ok(())
}

#[test]
fn active_class_allow_list_skips_unpromoted_intent_without_provider_calls()
-> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, candidates) = fixture(
        RerankLimits {
            input_cap: 4,
            score_cap: 2,
            output_cap: 2,
        },
        None,
        false,
        false,
    )?;
    reranker.set_allowed_intents(Some(BTreeSet::from([SearchIntent::RepositoryCode])))?;
    let baseline = candidates.clone();
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    assert_eq!(provider.query_calls(), 0);
    assert_eq!(provider.document_calls(), 0);
    assert_eq!(result.candidates, baseline);
    assert!(matches!(
        result.trace.candidates[0].position,
        maestria_domain::RerankPosition::SkippedNotApplicable
    ));
    Ok(())
}

#[test]
fn provider_receives_the_verified_evidence_excerpt() -> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, candidates) = fixture(
        RerankLimits {
            input_cap: 2,
            score_cap: 2,
            output_cap: 2,
        },
        None,
        false,
        false,
    )?;
    let _result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    assert_eq!(
        provider.document_texts(),
        Some(vec!["alpha".to_owned(), "beta".to_owned()])
    );
    Ok(())
}

#[test]
fn source_span_mismatch_falls_back_before_document_encoding()
-> Result<(), Box<dyn std::error::Error>> {
    let (reranker, provider, mut candidates) = fixture(
        RerankLimits {
            input_cap: 1,
            score_cap: 1,
            output_cap: 1,
        },
        None,
        false,
        false,
    )?;
    candidates[0].candidate = candidate_at_path(
        1,
        RetrievalReason::SemanticSimilarity,
        "different.txt".to_owned(),
    )?;
    let baseline = candidates.clone();
    let result = reranker.rerank(request(
        plan()?,
        candidates,
        Arc::new(SearchCancellation::new()),
        None,
    )?)?;
    assert_eq!(provider.query_calls(), 1);
    assert_eq!(provider.document_calls(), 0);
    assert_eq!(result.candidates, baseline);
    assert!(matches!(
        result.trace.candidates[0].position,
        maestria_domain::RerankPosition::ErrorFallback(ref code)
            if code == "late_interaction:source_invalid"
    ));
    Ok(())
}
