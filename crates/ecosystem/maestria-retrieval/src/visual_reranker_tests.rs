use std::collections::BTreeSet;
use std::sync::Arc;

use maestria_domain::{
    Artifact, ArtifactId, ArtifactVersionId, ContentRange, CorpusScope, CorpusSnapshotId, Evidence,
    EvidenceCandidate, EvidenceId, EvidenceKind, EvidenceRequirements, EvidenceSpan,
    FreshnessRequirement, FreshnessStatus, IndexGeneration, IndexGenerationId,
    IndexGenerationRegistry, IndexLifecycle, IndexStatus, Modality, ModalitySet, QueryId,
    RetrievalModelFingerprint, RetrievalReason, RetrievalScoreSet, SearchBudget, SearchIntent,
    SearchPlan, SearchStage, SecurityMetadata, SourceLocation, StopConditions, TrustLabel,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, EmbeddingIdentity, EmbeddingResponse, EvidenceRepository,
    InMemoryArtifactRepository, InMemoryBlobStore, InMemoryEvidenceRepository, PortError,
    ProviderDisclosure, RetentionPolicy, VisualEmbeddingProvider, VisualEmbeddingRequest,
};

use super::*;
use crate::types::{RankedCandidate, RerankLimits, RerankRequest};

struct FakeVisualProvider {
    identity: EmbeddingIdentity,
}

impl VisualEmbeddingProvider for FakeVisualProvider {
    fn disclosure(&self) -> Option<ProviderDisclosure> {
        Some(ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        })
    }

    fn embed_query(
        &self,
        _query: &str,
        identity: EmbeddingIdentity,
    ) -> Result<EmbeddingResponse, PortError> {
        Ok(response(identity, vec![1.0, 0.0]))
    }

    fn embed_source(
        &self,
        request: VisualEmbeddingRequest,
    ) -> Result<EmbeddingResponse, PortError> {
        let vector = if request.bytes.first() == Some(&2) {
            vec![1.0, 0.0]
        } else {
            vec![0.0, 1.0]
        };
        Ok(response(request.identity, vector))
    }

    fn identity(&self) -> Option<EmbeddingIdentity> {
        Some(self.identity.clone())
    }
}

fn response(identity: EmbeddingIdentity, vector: Vec<f32>) -> EmbeddingResponse {
    EmbeddingResponse {
        vector,
        provider_id: "fake-visual".to_string(),
        model: "fake-visual".to_string(),
        model_version: "1".to_string(),
        identity,
        disclosure: ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        },
    }
}

fn capability()
-> Result<(VisualGenerationCapability, EmbeddingIdentity), Box<dyn std::error::Error>> {
    let generation = IndexGenerationId::new(42);
    let snapshot = CorpusSnapshotId::new(7);
    let mut identity = EmbeddingIdentity::legacy("visual", 2)?;
    identity.generation_id = generation;
    identity.representation = maestria_domain::RepresentationName::new("visual_page_v1");
    let mut registry = IndexGenerationRegistry::default();
    registry.register(IndexGeneration {
        id: generation,
        name: maestria_domain::RepresentationName::new("visual_page_v1"),
        corpus_snapshot: snapshot,
        fingerprint: identity.fingerprint.clone(),
        lifecycle: IndexLifecycle::Building,
    })?;
    registry.transition_lifecycle(generation, IndexLifecycle::Evaluated)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Shadow)?;
    registry.transition_lifecycle(generation, IndexLifecycle::Active)?;
    let capability = VisualGenerationCapability::activate(&registry, identity.clone(), snapshot)?;
    Ok((capability, identity))
}

fn plan() -> Result<SearchPlan, Box<dyn std::error::Error>> {
    Ok(SearchPlan {
        query_id: QueryId::new(1),
        original_query: "show the figure".to_string(),
        intent: SearchIntent::VisualDocument,
        scope: CorpusScope::Global,
        corpus_snapshot: CorpusSnapshotId::new(7),
        index_generation: IndexGenerationId::new(42),
        freshness: FreshnessRequirement::Any,
        modalities: ModalitySet::new(vec![Modality::Text, Modality::Image]),
        stages: vec![SearchStage::InitialRetrieval, SearchStage::Reranking],
        budgets: SearchBudget::with_resource_limits(100, 100, 1, 2, 0, 0, 1)?,
        stop_conditions: StopConditions {
            max_results: 2,
            min_score_threshold: 0,
        },
        evidence_requirements: EvidenceRequirements {
            require_primary_sources: false,
            minimum_corroboration: 1,
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            minimum_sources: 0,
            minimum_documents: 0,
            minimum_sections: 0,
        },
        fingerprint: RetrievalModelFingerprint::new("test:visual".to_string())?,
    })
}

fn artifact(id: ArtifactId) -> Artifact {
    Artifact {
        id,
        title: "visual document".to_string(),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::Indexed,
        content_hash: None,
        parse_status: None,
        security: SecurityMetadata::default(),
    }
}

fn candidate(
    id: EvidenceId,
    page: u32,
    x: u32,
) -> Result<EvidenceCandidate, Box<dyn std::error::Error>> {
    Ok(EvidenceCandidate {
        evidence_id: id,
        artifact_version: ArtifactVersionId::new(1),
        source_span: EvidenceSpan::new(
            None,
            SourceLocation::Region {
                page,
                x,
                y: 2,
                width: 10,
                height: 10,
            },
            ContentRange { start: 0, end: 1 },
        )?,
        scores: RetrievalScoreSet {
            bm25: 1,
            semantic_similarity: 1,
        },
        trust: TrustLabel::Verified,
        freshness: FreshnessStatus::UpToDate,
        duplicate_cluster: None,
        reasons: vec![RetrievalReason::SemanticSimilarity],
        coverage_keys: Vec::new(),
    })
}

#[tokio::test]
async fn visual_reranker_reorders_visual_slots_and_preserves_coordinates()
-> Result<(), Box<dyn std::error::Error>> {
    let (capability, identity) = capability()?;
    let artifact_repo = Arc::new(InMemoryArtifactRepository::new());
    artifact_repo.put(artifact(ArtifactId::new(1)))?;
    let evidence_repo = Arc::new(InMemoryEvidenceRepository::new());
    let blob_store = Arc::new(InMemoryBlobStore::new());
    let blob_one = blob_store.put(vec![1])?;
    let blob_two = blob_store.put(vec![2])?;
    let first_id = EvidenceId::new(101);
    let second_id = EvidenceId::new(102);
    evidence_repo.put(Evidence {
        id: first_id,
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::PdfRegion {
            blob: blob_one,
            page: 1,
            x: 1,
            y: 2,
            width: 10,
            height: 10,
        },
        excerpt: "first figure".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: SecurityMetadata::default(),
    })?;
    evidence_repo.put(Evidence {
        id: second_id,
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::PdfRegion {
            blob: blob_two,
            page: 2,
            x: 20,
            y: 2,
            width: 10,
            height: 10,
        },
        excerpt: "second figure".to_string(),
        observed_at: maestria_domain::LogicalTick::new(1),
        security: SecurityMetadata::default(),
    })?;
    let reranker = VisualReranker::new(
        VisualRerankerParts {
            artifacts: artifact_repo,
            evidence: evidence_repo,
            blobs: blob_store,
            provider: Arc::new(FakeVisualProvider { identity }),
            capability,
            policy: RetrievalSecurityPolicy::default(),
        },
        RerankLimits {
            input_cap: 2,
            score_cap: 2,
            output_cap: 1,
        },
    )?;
    let first = candidate(first_id, 1, 1)?;
    let second = candidate(second_id, 2, 20)?;
    let result = reranker
        .rerank(RerankRequest {
            plan: plan()?,
            candidates: vec![
                RankedCandidate {
                    candidate: first,
                    rank: 0,
                },
                RankedCandidate {
                    candidate: second,
                    rank: 1,
                },
            ],
            max_latency_ms: 100,
        })
        .await?;
    assert_eq!(result.candidates[0].candidate.evidence_id, second_id);
    assert_eq!(result.candidates[1].candidate.evidence_id, first_id);
    assert_eq!(
        result.candidates[0].candidate.source_span.location(),
        &SourceLocation::Region {
            page: 2,
            x: 20,
            y: 2,
            width: 10,
            height: 10,
        }
    );
    assert!(result.trace.candidates.iter().any(|candidate| {
        candidate.candidate_id == second_id && candidate.status == RerankCandidateStatus::Reranked
    }));
    Ok(())
}

#[tokio::test]
async fn visual_reranker_returns_traced_fallback_for_secret_queries()
-> Result<(), Box<dyn std::error::Error>> {
    let (capability, identity) = capability()?;
    let reranker = VisualReranker::new(
        VisualRerankerParts {
            artifacts: Arc::new(InMemoryArtifactRepository::new()),
            evidence: Arc::new(InMemoryEvidenceRepository::new()),
            blobs: Arc::new(InMemoryBlobStore::new()),
            provider: Arc::new(FakeVisualProvider { identity }),
            capability,
            policy: RetrievalSecurityPolicy::default(),
        },
        RerankLimits {
            input_cap: 1,
            score_cap: 1,
            output_cap: 1,
        },
    )?;
    let mut secret_plan = plan()?;
    secret_plan.original_query = "password=not-for-search".to_string();
    let evidence_id = EvidenceId::new(103);
    let result = reranker
        .rerank(RerankRequest {
            plan: secret_plan,
            candidates: vec![RankedCandidate {
                candidate: candidate(evidence_id, 1, 1)?,
                rank: 0,
            }],
            max_latency_ms: 100,
        })
        .await?;
    assert_eq!(result.candidates[0].candidate.evidence_id, evidence_id);
    assert!(matches!(
        result.trace.candidates[0].status,
        RerankCandidateStatus::ErrorFallback(_)
    ));
    Ok(())
}

#[test]
fn visual_reranker_cosine_rejects_incompatible_vectors() {
    assert_eq!(VisualReranker::cosine(&[1.0], &[1.0, 2.0]), None);
}
