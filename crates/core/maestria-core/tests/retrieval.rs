use maestria_core::{
    CorePorts, CoreServices, IngestFileInput, OpenChunkEvidenceInput, SearchInput,
};
use maestria_domain::{ArtifactId, EvidenceKind, LogicalTick};
use maestria_ports::{
    FileHandle, FileMetadata, InMemoryArtifactRepository, InMemoryBlobStore,
    InMemoryCardRepository, InMemoryChunkRepository, InMemoryEventLog, InMemoryEvidenceRepository,
    InMemoryFullTextIndex, ParseContext, ParsedArtifact, ParsedChunk, Parser, PortError,
};

#[derive(Clone)]
struct ParagraphParser;

impl Parser for ParagraphParser {
    fn id(&self) -> &'static str {
        "paragraph-parser"
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        file.extension.as_deref() == Some("md")
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let text = String::from_utf8(file.bytes).map_err(|err| PortError::InvalidInput {
            message: format!("file bytes are not utf8: {err}"),
        })?;
        let mut chunks = Vec::new();
        for paragraph in text.split("\n\n").filter(|paragraph| !paragraph.is_empty()) {
            let chunk_index = chunks.len() as u64;
            chunks.push(ParsedChunk {
                chunk_id: maestria_domain::ChunkId::new(
                    context
                        .artifact_id
                        .value()
                        .saturating_mul(100)
                        .saturating_add(chunk_index)
                        .saturating_add(1),
                ),
                artifact_id: context.artifact_id,
                text: paragraph.to_string(),
            });
        }

        Ok(ParsedArtifact {
            artifact_id: context.artifact_id,
            chunks,
            cards: Vec::new(),
        })
    }
}

#[test]
fn chunk_evidence_lookup_uses_matching_chunk_order_and_source_span()
-> Result<(), Box<dyn std::error::Error>> {
    let artifacts = InMemoryArtifactRepository::new();
    let chunks = InMemoryChunkRepository::new();
    let cards = InMemoryCardRepository::new();
    let evidence = InMemoryEvidenceRepository::new();
    let events = InMemoryEventLog::new();
    let parser = ParagraphParser;
    let search_index = InMemoryFullTextIndex::new();
    let blobs = InMemoryBlobStore::new();
    let core = CoreServices::new(CorePorts {
        artifacts: &artifacts,
        chunks: &chunks,
        cards: &cards,
        evidence: &evidence,
        events: &events,
        parser: &parser,
        search_index: &search_index,
        blobs: &blobs,
    });

    let ingested = core.ingest_file_from_bytes(IngestFileInput {
        path: std::path::PathBuf::from("notes/multi-source.md"),
        bytes: concat!(
            "Alpha source span anchors first evidence.\n",
            "\n",
            "Beta source span carries beta-token evidence.\n",
            "\n",
            "Gamma source span carries gamma-token evidence.\n",
        )
        .as_bytes()
        .to_vec(),
        observed_at: LogicalTick::new(11),
        artifact_id: Some(ArtifactId::new(7)),
    })?;

    let expected = [
        ("Alpha source span anchors first evidence.", 1usize),
        ("Beta source span carries beta-token evidence.", 3usize),
        ("Gamma source span carries gamma-token evidence.", 5usize),
    ];
    assert_eq!(ingested.chunks.len(), expected.len());
    assert_eq!(ingested.evidence.len(), expected.len());

    for (order, ((chunk, evidence), (excerpt, line))) in ingested
        .chunks
        .iter()
        .zip(ingested.evidence.iter())
        .zip(expected.iter())
        .enumerate()
    {
        assert_eq!(chunk.order, order as u32);
        assert_eq!(evidence.excerpt, *excerpt);

        let opened = core.open_chunk_evidence(OpenChunkEvidenceInput { chunk_id: chunk.id })?;
        assert_eq!(opened.artifact.id, ingested.artifact.id);
        assert_eq!(opened.evidence.id, evidence.id);
        assert_eq!(opened.evidence.excerpt, *excerpt);
        match opened.evidence.kind {
            EvidenceKind::FileSpan {
                path,
                range,
                content_hash,
                ..
            } => {
                assert_eq!(path, "notes/multi-source.md");
                assert_eq!(range.start, *line);
                assert_eq!(range.end, *line);
                assert_eq!(content_hash, ingested.content_hash);
            }
            other => panic!("expected file evidence, got {other:?}"),
        }
    }

    let search = core.search(SearchInput {
        query: "gamma-token".to_string(),
        limit: 5,
    })?;
    assert_eq!(search.hits.len(), 1);
    let hit = &search.hits[0];
    assert_eq!(hit.artifact.id, ingested.artifact.id);
    assert_eq!(hit.chunk.id, ingested.chunks[2].id);
    let hit_evidence = &hit.evidence;
    assert_eq!(hit_evidence.id, ingested.evidence[2].id);
    assert_eq!(hit_evidence.excerpt, expected[2].0);
    match &hit_evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
            ..
        } => {
            assert_eq!(path, "notes/multi-source.md");
            assert_eq!(range.start, expected[2].1);
            assert_eq!(range.end, expected[2].1);
            assert_eq!(content_hash, &ingested.content_hash);
        }
        other => panic!("expected file evidence, got {other:?}"),
    }

    Ok(())
}
