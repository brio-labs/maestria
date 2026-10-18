use std::collections::BTreeSet;
use std::sync::Arc;

use sillage_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, Evidence, EvidenceKind, IndexGenerationId,
    IndexStatus, LineRange, LogicalTick, SearchIntent, SearchLaneStatus, SecurityMetadata,
    SnapshotRef, SourceLocation, SourceSpan, StructureNodeId, content_hash, evidence_id_for,
    representations_digest,
};
use sillage_ports::{
    ArtifactRepository, BlobStore, CardRepository, ChunkRepository, EvidenceRepository,
    FullTextIndex, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEvidenceRepository, IndexedCard,
};
use sillage_search_tantivy::TantivyFullTextIndex;

use crate::adapters::filtered_test_support::request;
use crate::adapters::{CardRetriever, CardRetrieverParts};
use crate::traits::CandidateRetriever;

const ARTIFACT: ArtifactId = ArtifactId::new(41);
const CARD: CardId = CardId::new(81);
const PATH: &str = "notes/ledgeranchor.md";
const FIRST: &str = "A brass route is documented in this immutable source.";
const SECOND: &str = "The second source paragraph describes an oak chart.";
type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

fn stored_chunks(
    spans: &[SourceSpan],
    snapshot: &SnapshotRef,
    evidence_read_allowed: bool,
) -> TestResult<(
    Arc<InMemoryChunkRepository>,
    Arc<InMemoryEvidenceRepository>,
)> {
    let chunks = Arc::new(InMemoryChunkRepository::new());
    let evidence = Arc::new(InMemoryEvidenceRepository::new());
    for (order, span) in spans.iter().copied().enumerate() {
        let order = u32::try_from(order)?;
        let line = if span == SourceSpan::text_span(2, 2)? {
            2
        } else {
            1
        };
        let text = if line == 2 { SECOND } else { FIRST };
        chunks.put(Chunk {
            id: ChunkId::new(u64::from(order) + 1),
            artifact_id: ARTIFACT,
            node_id: StructureNodeId::new(u64::from(order) + 2),
            source_span: span,
            representations: Vec::new(),
            representations_digest: representations_digest(&[]),
            order,
            text: text.to_owned(),
        })?;
        evidence.put(Evidence {
            id: evidence_id_for(ARTIFACT, order),
            artifact_id: ARTIFACT,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: PATH.to_owned(),
                range: LineRange::new(line, line)?,
                snapshot: snapshot.clone(),
            },
            excerpt: text.to_owned(),
            observed_at: LogicalTick::new(1),
            security: SecurityMetadata {
                read_allowed: evidence_read_allowed,
                ..SecurityMetadata::default()
            },
        })?;
    }
    Ok((chunks, evidence))
}

fn records(
    index: Arc<TantivyFullTextIndex>,
    spans: &[SourceSpan],
    evidence_read_allowed: bool,
) -> TestResult<CardRetrieverParts> {
    let blobs = Arc::new(InMemoryBlobStore::new());
    let source = format!("{FIRST}\n{SECOND}").into_bytes();
    let hash = sillage_domain::ContentHash::new(content_hash(&source))?;
    let snapshot = SnapshotRef::new(blobs.put(source)?, hash.clone());
    let (chunks, evidence) = stored_chunks(spans, &snapshot, evidence_read_allowed)?;
    let artifacts = Arc::new(InMemoryArtifactRepository::new());
    artifacts.put(Artifact {
        id: ARTIFACT,
        title: PATH.to_owned(),
        chunk_ids: (0..spans.len())
            .map(|order| ChunkId::new(order as u64 + 1))
            .collect(),
        card_ids: BTreeSet::from([CARD]),
        claim_ids: BTreeSet::new(),
        evidence_ids: (0..spans.len())
            .map(|order| evidence_id_for(ARTIFACT, order as u32))
            .collect(),
        index_status: IndexStatus::Indexed,
        content_hash: Some(hash),
        parse_status: None,
        security: SecurityMetadata::default(),
    })?;
    let cards = Arc::new(InMemoryCardRepository::new());
    cards.put(Card {
        id: CARD,
        artifact_id: ARTIFACT,
        node_id: StructureNodeId::new(100),
        source_span: SourceSpan::text_span(1, 1)?,
        title: "ledgeranchor".to_owned(),
        body: PATH.to_owned(),
        security: SecurityMetadata::default(),
    })?;
    Ok(CardRetrieverParts {
        index,
        artifacts,
        cards,
        chunks,
        evidence,
        blobs,
    })
}

fn retrieve(
    spans: &[SourceSpan],
    evidence_read_allowed: bool,
) -> TestResult<crate::types::CandidateBatch> {
    let directory = tempfile::tempdir()?;
    let index = Arc::new(TantivyFullTextIndex::open(directory.path())?);
    index.index_cards(vec![IndexedCard {
        artifact_id: ARTIFACT,
        card_id: CARD,
        title: "ledgeranchor".to_owned(),
        body: PATH.to_owned(),
    }])?;
    index.commit_and_reload()?;
    let retriever = CardRetriever::new(
        records(index, spans, evidence_read_allowed)?,
        IndexGenerationId::new(1),
    );
    let mut request = request(SearchIntent::FactualLocal, IndexGenerationId::new(1))?;
    request.query.q = "ledgeranchor".to_owned();
    request.authorization = sillage_governance::RetrievalSecurityPolicy::default()
        .allow_unscoped_items(true)
        .authorization_context(request.plan.scope())?;
    Ok(retriever.retrieve(request)?)
}

#[test]
fn structural_card_returns_its_exact_source_chunk() -> TestResult<()> {
    let batch = retrieve(&[SourceSpan::text_span(1, 1)?], true)?;
    assert_eq!(batch.status, SearchLaneStatus::Succeeded);
    assert_eq!(batch.candidates.len(), 1);
    let candidate = &batch.candidates[0];
    assert_eq!(candidate.evidence_id(), evidence_id_for(ARTIFACT, 0));
    assert_eq!(
        candidate.source_span().node_id(),
        Some(StructureNodeId::new(2))
    );
    assert_eq!(
        candidate.source_span().location(),
        &SourceLocation::file(PATH.to_owned(), 1, 1)?
    );
    Ok(())
}

#[test]
fn structural_card_rejects_missing_or_ambiguous_source_spans() -> TestResult<()> {
    let first = SourceSpan::text_span(1, 1)?;
    for spans in [vec![SourceSpan::text_span(2, 2)?], vec![first, first]] {
        let batch = retrieve(&spans, true)?;
        assert_eq!(batch.status, SearchLaneStatus::Empty);
        assert!(batch.candidates.is_empty());
    }
    Ok(())
}

#[test]
fn structural_card_does_not_expose_denied_chunk_evidence() -> TestResult<()> {
    let batch = retrieve(&[SourceSpan::text_span(1, 1)?], false)?;
    assert_eq!(batch.status, SearchLaneStatus::Empty);
    assert!(batch.candidates.is_empty());
    Ok(())
}
