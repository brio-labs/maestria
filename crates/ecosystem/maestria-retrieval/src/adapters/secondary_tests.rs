use super::*;
use maestria_domain::{
    Artifact, ArtifactId, Chunk, ChunkId, ContentHash, Evidence, EvidenceId, EvidenceKind,
    LogicalTick, RelationKind, SourceSpan, StructureNodeId, ValidationReportId,
};
use maestria_governance::RetrievalSecurityPolicy;
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, GraphIndex,
    InMemoryArtifactRepository, InMemoryChunkRepository, InMemoryEvidenceRepository,
    InMemoryGraphIndex, PortError,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct CountingBlobStore {
    bytes: Vec<u8>,
    gets: Arc<AtomicUsize>,
}

impl BlobStore for CountingBlobStore {
    fn put(&self, _bytes: Vec<u8>) -> Result<maestria_domain::BlobId, PortError> {
        Ok(maestria_domain::BlobId::new(1))
    }

    fn get(&self, _id: maestria_domain::BlobId) -> Result<Vec<u8>, PortError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        Ok(self.bytes.clone())
    }
}

struct DeniedRelationArtifacts {
    owner_id: ArtifactId,
    neighbor_id: ArtifactId,
    repository: Arc<InMemoryArtifactRepository>,
}

struct DeniedRelationRecords {
    evidence: Arc<InMemoryEvidenceRepository>,
    chunks: Arc<InMemoryChunkRepository>,
    graph: Arc<InMemoryGraphIndex>,
}

struct DeniedRelationFixture {
    expander: HierarchyGraphExpander,
    seed: EvidenceCandidate,
    blob_gets: Arc<AtomicUsize>,
}

fn denied_relation_artifacts(
    content_hash: &str,
    owner_read_allowed: bool,
) -> Result<DeniedRelationArtifacts, Box<dyn std::error::Error>> {
    let owner_id = ArtifactId::new(1);
    let neighbor_id = ArtifactId::new(2);
    let repository = Arc::new(InMemoryArtifactRepository::new());
    repository.put(Artifact {
        id: owner_id,
        title: "denied owner".to_string(),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: maestria_domain::IndexStatus::Indexed,
        content_hash: Some(ContentHash::new(content_hash.to_string())?),
        parse_status: None,
        security: maestria_domain::SecurityMetadata {
            read_allowed: owner_read_allowed,
            ..Default::default()
        },
    })?;
    repository.put(Artifact {
        id: neighbor_id,
        title: "neighbor".to_string(),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: maestria_domain::IndexStatus::Indexed,
        content_hash: Some(ContentHash::new(content_hash.to_string())?),
        parse_status: None,
        security: Default::default(),
    })?;
    Ok(DeniedRelationArtifacts {
        owner_id,
        neighbor_id,
        repository,
    })
}

fn denied_relation_records(
    artifacts: &DeniedRelationArtifacts,
    content_hash: &str,
    chunk_text: &str,
) -> Result<DeniedRelationRecords, Box<dyn std::error::Error>> {
    let relation_evidence_id = EvidenceId::new(7);
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    evidence.put(Evidence {
        id: relation_evidence_id,
        artifact_id: artifacts.owner_id,
        claim_id: None,
        kind: EvidenceKind::WebSnapshot {
            url: "https://example.test/relation".to_string(),
            snapshot: maestria_domain::SnapshotRef::new(
                maestria_domain::BlobId::new(1),
                ContentHash::new(content_hash.to_string())?,
            ),
            fetched_at: LogicalTick::new(1),
            metadata: Default::default(),
        },
        excerpt: "relation and target evidence".to_string(),
        observed_at: LogicalTick::new(1),
        security: Default::default(),
    })?;
    evidence.put(Evidence {
        id: EvidenceId::new(8),
        artifact_id: artifacts.neighbor_id,
        claim_id: None,
        kind: EvidenceKind::WebSnapshot {
            url: "https://example.test/target".to_string(),
            snapshot: maestria_domain::SnapshotRef::new(
                maestria_domain::BlobId::new(1),
                ContentHash::new(content_hash.to_string())?,
            ),
            fetched_at: LogicalTick::new(1),
            metadata: Default::default(),
        },
        excerpt: "relation and target evidence".to_string(),
        observed_at: LogicalTick::new(1),
        security: Default::default(),
    })?;

    let chunks = Arc::new(InMemoryChunkRepository::new());
    chunks.put(Chunk {
        id: ChunkId::new(2),
        artifact_id: artifacts.neighbor_id,
        node_id: StructureNodeId::new(2),
        source_span: SourceSpan::text_span(1, 1)?,
        representations: Vec::new(),
        order: 1,
        text: chunk_text.to_string(),
    })?;

    let graph = Arc::new(InMemoryGraphIndex::new());
    graph.insert_relation(maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: RelationEndpoint::Artifact(artifacts.owner_id),
        kind: RelationKind::RelatedTo,
        target: RelationEndpoint::Artifact(artifacts.neighbor_id),
        evidence_id: Some(relation_evidence_id),
        confidence_milli: 1_000,
        security: Default::default(),
    })?;
    Ok(DeniedRelationRecords {
        evidence,
        chunks,
        graph,
    })
}

fn denied_relation_expander_and_seed(
    artifacts: DeniedRelationArtifacts,
    records: DeniedRelationRecords,
    blobs: Arc<CountingBlobStore>,
    blob_gets: Arc<AtomicUsize>,
) -> Result<DeniedRelationFixture, Box<dyn std::error::Error>> {
    let expander = HierarchyGraphExpander::new(HierarchyGraphExpanderParts {
        graph: records.graph,
        artifacts: artifacts.repository,
        chunks: records.chunks,
        evidence: records.evidence,
        blobs,
    });
    let seed_evidence = Evidence {
        id: EvidenceId::new(99),
        artifact_id: artifacts.owner_id,
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "seed".to_string(),
        observed_at: LogicalTick::new(1),
        security: Default::default(),
    };
    let seed = candidate_from_records(
        artifacts.owner_id,
        &SourceSpan::text_span(1, 1)?,
        &seed_evidence,
        StructureNodeId::new(1),
        maestria_domain::RetrievalScoreSet::empty(),
        Vec::new(),
    )?;
    Ok(DeniedRelationFixture {
        expander,
        seed,
        blob_gets,
    })
}

#[test]
fn denied_relation_owner_causes_zero_blob_reads_and_zero_expansion()
-> Result<(), Box<dyn std::error::Error>> {
    let source = b"relation and target evidence\n";
    let content_hash = maestria_domain::content_hash(source);
    let blob_gets = Arc::new(AtomicUsize::new(0));
    let blobs = Arc::new(CountingBlobStore {
        bytes: source.to_vec(),
        gets: Arc::clone(&blob_gets),
    });
    let artifacts = denied_relation_artifacts(&content_hash, false)?;
    let records = denied_relation_records(&artifacts, &content_hash, "neighbor")?;
    let fixture = denied_relation_expander_and_seed(artifacts, records, blobs, blob_gets)?;
    let authorization = RetrievalSecurityPolicy::default()
        .authorization_context(&maestria_domain::CorpusScope::Global)?;
    let expanded = fixture.expander.expand(
        &[RankedCandidate {
            candidate: fixture.seed,
            rank: 1,
        }],
        &ExpansionPolicy {
            max_results: 3,
            max_depth: 2,
            selected_seeds: Vec::new(),
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            authorization,
            execution_budget: maestria_domain::SearchExecutionBudget::new(3, 3, 3, 0)?,
        },
    )?;

    assert_eq!(expanded.candidates.len(), 1);
    assert_eq!(fixture.blob_gets.load(Ordering::Relaxed), 0);
    Ok(())
}

#[test]
fn graph_expansion_rejects_secret_bearing_chunks() -> Result<(), Box<dyn std::error::Error>> {
    let source = b"relation and target evidence\n";
    let content_hash = maestria_domain::content_hash(source);
    let blob_gets = Arc::new(AtomicUsize::new(0));
    let blobs = Arc::new(CountingBlobStore {
        bytes: source.to_vec(),
        gets: Arc::clone(&blob_gets),
    });
    let artifacts = denied_relation_artifacts(&content_hash, true)?;
    let records =
        denied_relation_records(&artifacts, &content_hash, "password=super-secret-value")?;
    let fixture = denied_relation_expander_and_seed(artifacts, records, blobs, blob_gets)?;
    let authorization = RetrievalSecurityPolicy::default()
        .authorization_context(&maestria_domain::CorpusScope::Global)?;
    let expanded = fixture.expander.expand(
        &[RankedCandidate {
            candidate: fixture.seed,
            rank: 1,
        }],
        &ExpansionPolicy {
            max_results: 3,
            max_depth: 2,
            selected_seeds: Vec::new(),
            required_claims: Vec::new(),
            required_subquestions: Vec::new(),
            authorization,
            execution_budget: maestria_domain::SearchExecutionBudget::new(3, 3, 3, 0)?,
        },
    )?;

    assert_eq!(expanded.candidates.len(), 1);
    assert_eq!(fixture.blob_gets.load(Ordering::Relaxed), 2);
    Ok(())
}
