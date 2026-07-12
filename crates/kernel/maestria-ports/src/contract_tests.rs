use super::*;
use maestria_domain::{
    ClaimId, ContentRange, EventId, EvidenceKind, LogicalTick, RelationKind, SequenceNumber,
    ValidationReportId,
};

pub fn sample_artifact(id: u64) -> Artifact {
    Artifact {
        id: ArtifactId::new(id),
        title: format!("artifact-{id}"),
        chunk_ids: Default::default(),
        card_ids: Default::default(),
        claim_ids: Default::default(),
        evidence_ids: Default::default(),
        index_status: Default::default(),
        content_hash: None,
    }
}

pub fn assert_artifact_repository_round_trip(repository: &impl ArtifactRepository) {
    let artifact = sample_artifact(1);

    repository.put(artifact.clone()).expect("artifact put");

    assert_eq!(
        repository.get(artifact.id).expect("artifact get"),
        Some(artifact)
    );
    assert_eq!(
        repository
            .get(ArtifactId::new(99))
            .expect("missing artifact get"),
        None
    );
}

pub fn assert_chunk_repository_round_trip(repository: &impl ChunkRepository) {
    let first = Chunk {
        id: ChunkId::new(10),
        artifact_id: ArtifactId::new(1),
        order: 2,
        text: "second".to_string(),
    };
    let second = Chunk {
        id: ChunkId::new(11),
        artifact_id: ArtifactId::new(1),
        order: 1,
        text: "first".to_string(),
    };
    let unrelated = Chunk {
        id: ChunkId::new(12),
        artifact_id: ArtifactId::new(2),
        order: 0,
        text: "other".to_string(),
    };

    repository.put(first.clone()).expect("first chunk put");
    repository.put(second.clone()).expect("second chunk put");
    repository.put(unrelated).expect("unrelated chunk put");

    assert_eq!(
        repository.get(first.id).expect("chunk get"),
        Some(first.clone())
    );
    assert_eq!(
        repository
            .list_for_artifact(ArtifactId::new(1))
            .expect("chunk list"),
        vec![second, first]
    );
    assert_eq!(
        repository.get(ChunkId::new(99)).expect("missing chunk get"),
        None
    );
}

pub fn assert_card_repository_round_trip(repository: &impl CardRepository) {
    let first = Card {
        id: CardId::new(20),
        artifact_id: ArtifactId::new(1),
        title: "bravo".to_string(),
        body: "body b".to_string(),
        claim_ids: [ClaimId::new(3), ClaimId::new(1)].into(),
    };
    let second = Card {
        id: CardId::new(21),
        artifact_id: ArtifactId::new(1),
        title: "alpha".to_string(),
        body: "body a".to_string(),
        claim_ids: Default::default(),
    };
    let unrelated = Card {
        id: CardId::new(22),
        artifact_id: ArtifactId::new(2),
        title: "other".to_string(),
        body: "body".to_string(),
        claim_ids: Default::default(),
    };

    repository.put(first.clone()).expect("first card put");
    repository.put(second.clone()).expect("second card put");
    repository.put(unrelated).expect("unrelated card put");

    assert_eq!(
        repository.get(first.id).expect("card get"),
        Some(first.clone())
    );
    assert_eq!(
        repository
            .list_for_artifact(ArtifactId::new(1))
            .expect("card list"),
        vec![first, second]
    );
    assert_eq!(
        repository.get(CardId::new(99)).expect("missing card get"),
        None
    );
}

pub fn assert_evidence_repository_round_trip(repository: &impl EvidenceRepository) {
    let file = Evidence {
        id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(7)),
        kind: EvidenceKind::FileSpan {
            path: "notes.md".to_string(),
            range: ContentRange { start: 1, end: 4 },
            content_hash: "sha256:notes".to_string(),
            snapshot: None,
        },
        excerpt: "source excerpt".to_string(),
        observed_at: LogicalTick::new(9),
    };
    let validation = Evidence {
        id: EvidenceId::new(41),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(5),
        },
        excerpt: "validated".to_string(),
        observed_at: LogicalTick::new(10),
    };
    let unrelated = Evidence {
        id: EvidenceId::new(42),
        artifact_id: ArtifactId::new(2),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(6),
        },
        excerpt: "other".to_string(),
        observed_at: LogicalTick::new(11),
    };

    repository.put(file.clone()).expect("file evidence put");
    repository
        .put(validation.clone())
        .expect("validation evidence put");
    repository.put(unrelated).expect("unrelated evidence put");

    assert_eq!(
        repository.get(file.id).expect("evidence get"),
        Some(file.clone())
    );
    assert_eq!(
        repository
            .list_for_artifact(ArtifactId::new(1))
            .expect("evidence list"),
        vec![file, validation]
    );
    assert_eq!(
        repository
            .get(EvidenceId::new(99))
            .expect("missing evidence get"),
        None
    );
}

pub fn assert_evidence_repository_replace_contract(repository: &impl EvidenceRepository) {
    let original = Evidence {
        id: EvidenceId::new(50),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(1),
        },
        excerpt: "original excerpt".to_string(),
        observed_at: LogicalTick::new(1),
    };
    let replacement = Evidence {
        id: EvidenceId::new(50),         // same id
        artifact_id: ArtifactId::new(2), // different artifact
        claim_id: Some(ClaimId::new(9)),
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(2),
        },
        excerpt: "replacement excerpt".to_string(),
        observed_at: LogicalTick::new(2),
    };

    repository.put(original.clone()).expect("original put");
    // put with different content must conflict
    let err = repository.put(replacement.clone()).unwrap_err();
    assert!(matches!(err, PortError::Conflict { .. }));
    // original still intact
    assert_eq!(
        repository.get(original.id).expect("get"),
        Some(original.clone())
    );
    // replace overwrites even with different content
    repository
        .replace(replacement.clone())
        .expect("replace must succeed despite conflict");
    assert_eq!(
        repository.get(replacement.id).expect("get after replace"),
        Some(replacement.clone())
    );
    // replace of identical value is idempotent
    repository
        .replace(replacement.clone())
        .expect("replace identical must succeed");
    assert_eq!(
        repository
            .get(replacement.id)
            .expect("get after replace identical"),
        Some(replacement.clone())
    );
    // replace on a fresh id acts as insert
    let fresh = Evidence {
        id: EvidenceId::new(51),
        artifact_id: ArtifactId::new(1),
        claim_id: None,
        kind: EvidenceKind::Validation {
            report_id: ValidationReportId::new(3),
        },
        excerpt: "fresh".to_string(),
        observed_at: LogicalTick::new(3),
    };
    repository.replace(fresh.clone()).expect("fresh replace");
    assert_eq!(repository.get(fresh.id).expect("get fresh"), Some(fresh));
}

pub fn assert_event_log_round_trip(log: &impl EventLog) {
    let event = DomainEventEnvelope {
        id: EventId::new(1),
        sequence: SequenceNumber::new(1),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(1),
            title: "notes".to_string(),
        },
    };
    let evidence = DomainEventEnvelope {
        id: EventId::new(2),
        sequence: SequenceNumber::new(2),
        event: DomainEvent::EvidenceRecorded {
            evidence_id: EvidenceId::new(40),
            artifact_id: ArtifactId::new(1),
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: "notes.md".to_string(),
                range: ContentRange { start: 1, end: 4 },
                content_hash: "sha256:notes".to_string(),
                snapshot: None,
            },
            excerpt: "excerpt".to_string(),
            observed_at: LogicalTick::new(0),
        },
    };
    let search = DomainEventEnvelope {
        id: EventId::new(3),
        sequence: SequenceNumber::new(3),
        event: DomainEvent::SearchCompleted {
            artifact_id: ArtifactId::new(1),
            cards_added: 2,
        },
    };
    let unrelated = DomainEventEnvelope {
        id: EventId::new(4),
        sequence: SequenceNumber::new(4),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(2),
            title: "other".to_string(),
        },
    };

    log.append(event.clone()).expect("event append");
    log.append(evidence.clone()).expect("evidence event append");
    log.append(search.clone()).expect("search event append");
    log.append(unrelated).expect("unrelated event append");

    let out_of_order = DomainEventEnvelope {
        id: EventId::new(6), // next is 5
        sequence: SequenceNumber::new(6),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(0),
        },
    };
    let err = log.append(out_of_order).unwrap_err();
    assert!(
        matches!(err, PortError::Conflict { .. }),
        "out of order must return Conflict"
    );

    let id_mismatch = DomainEventEnvelope {
        id: EventId::new(99),
        sequence: SequenceNumber::new(5),
        event: DomainEvent::TickObserved {
            at: LogicalTick::new(0),
        },
    };
    let err_id = log.append(id_mismatch).unwrap_err();
    assert!(
        matches!(err_id, PortError::Conflict { .. }),
        "id mismatch must return Conflict"
    );

    let all = log
        .scan(EventFilter { artifact_id: None })
        .expect("full event scan");
    assert_eq!(all.len(), 4);

    let filtered = log
        .scan(EventFilter {
            artifact_id: Some(ArtifactId::new(1)),
        })
        .expect("filtered event scan");
    assert_eq!(filtered, vec![event, evidence, search]);
}

pub fn assert_blob_store_round_trip(store: &impl BlobStore) {
    let first = store.put(vec![1, 2, 3]).expect("first blob put");
    let first_duplicate = store.put(vec![1, 2, 3]).expect("duplicate blob put");
    let second = store.put(vec![4, 5]).expect("second blob put");

    assert_eq!(first, first_duplicate);
    assert_ne!(first, second);
    assert_eq!(store.get(first).expect("first blob get"), vec![1, 2, 3]);
    assert_eq!(store.get(second).expect("second blob get"), vec![4, 5]);
    assert!(matches!(
        store.get(BlobId::new(99)),
        Err(PortError::NotFound)
    ));
}

pub fn assert_full_text_index_round_trip(index: &impl FullTextIndex) {
    // --- chunk round-trip (existing) ---
    index
        .index_chunks(vec![
            IndexedChunk {
                artifact_id: ArtifactId::new(1),
                chunk_id: ChunkId::new(10),
                text: "hello short".to_string(),
            },
            IndexedChunk {
                artifact_id: ArtifactId::new(1),
                chunk_id: ChunkId::new(11),
                text: "hello search with more ranking text".to_string(),
            },
            IndexedChunk {
                artifact_id: ArtifactId::new(2),
                chunk_id: ChunkId::new(20),
                text: "unrelated".to_string(),
            },
        ])
        .expect("index chunks");

    let hits = index
        .search(SearchQuery {
            q: "hello".to_string(),
            limit: 1,
        })
        .expect("search hits");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].chunk.chunk_id, ChunkId::new(11));

    // --- card round-trip ---
    index
        .index_cards(vec![
            IndexedCard {
                artifact_id: ArtifactId::new(1),
                card_id: CardId::new(100),
                title: "Alpha".to_string(),
                body: "first card".to_string(),
            },
            IndexedCard {
                artifact_id: ArtifactId::new(1),
                card_id: CardId::new(101),
                title: "Beta".to_string(),
                body: "second card with more content".to_string(),
            },
            IndexedCard {
                artifact_id: ArtifactId::new(2),
                card_id: CardId::new(200),
                title: "Gamma".to_string(),
                body: "unrelated".to_string(),
            },
        ])
        .expect("index cards");

    let card_hits = index
        .search_cards(SearchQuery {
            q: "card".to_string(),
            limit: 10,
        })
        .expect("search cards");

    assert_eq!(card_hits.len(), 2);
    // Beta has longer body, so it should rank above Alpha
    assert_eq!(card_hits[0].card.card_id, CardId::new(101));
    assert_eq!(card_hits[1].card.card_id, CardId::new(100));

    // --- card replacement: re-index card 100 with updated content ---
    index
        .index_cards(vec![IndexedCard {
            artifact_id: ArtifactId::new(1),
            card_id: CardId::new(100),
            title: "Alpha Updated".to_string(),
            body: "revised first card".to_string(),
        }])
        .expect("index replacement cards");

    // Old Beta (card_id=101) should still exist — only card 100 was re-indexed.
    let beta_hits = index
        .search_cards(SearchQuery {
            q: "second".to_string(),
            limit: 10,
        })
        .expect("search after replace");
    assert_eq!(beta_hits.len(), 1);
    assert_eq!(beta_hits[0].card.card_id, CardId::new(101));

    let updated_hits = index
        .search_cards(SearchQuery {
            q: "revised".to_string(),
            limit: 10,
        })
        .expect("search revised");
    assert_eq!(updated_hits.len(), 1);
    assert_eq!(updated_hits[0].card.card_id, CardId::new(100));
    assert_eq!(updated_hits[0].card.title, "Alpha Updated");

    // --- deterministic tie ordering: same scores, ordered by (artifact_id, card_id) ---
    index
        .index_cards(vec![
            IndexedCard {
                artifact_id: ArtifactId::new(3),
                card_id: CardId::new(301),
                title: "dup".to_string(),
                body: "same".to_string(),
            },
            IndexedCard {
                artifact_id: ArtifactId::new(3),
                card_id: CardId::new(302),
                title: "dup".to_string(),
                body: "same".to_string(),
            },
            IndexedCard {
                artifact_id: ArtifactId::new(3),
                card_id: CardId::new(303),
                title: "dup".to_string(),
                body: "same".to_string(),
            },
        ])
        .expect("index tie cards");

    let tie_hits = index
        .search_cards(SearchQuery {
            q: "dup".to_string(),
            limit: 10,
        })
        .expect("search ties");

    // All three should match; order must be by ascending card_id for ties
    let tie_ids: Vec<CardId> = tie_hits.iter().map(|h| h.card.card_id).collect();
    assert_eq!(
        tie_ids,
        vec![CardId::new(301), CardId::new(302), CardId::new(303)]
    );

    // --- empty query returns empty ---
    let empty = index
        .search_cards(SearchQuery {
            q: "zzz_no_match".to_string(),
            limit: 10,
        })
        .expect("empty search");
    assert!(empty.is_empty());
}

pub fn assert_vector_index_contract(index: &impl VectorIndex) {
    let prov = || EmbeddingProvenance {
        content_hash: "abcd123".into(),
        model_version: "test-v1".into(),
    };

    index
        .index_embeddings(vec![
            VectorEmbedding {
                chunk_id: ChunkId::new(2),
                vector: vec![1.0, 0.0],
                provenance: prov(),
            },
            VectorEmbedding {
                chunk_id: ChunkId::new(1),
                vector: vec![1.0, 0.0],
                provenance: prov(),
            },
            VectorEmbedding {
                chunk_id: ChunkId::new(3),
                vector: vec![0.0, 1.0],
                provenance: prov(),
            },
            VectorEmbedding {
                chunk_id: ChunkId::new(4),
                vector: vec![1.0, 0.0, 0.0],
                provenance: prov(),
            },
        ])
        .expect("index embeddings");

    let equal_score_hits = index
        .search_similar(VectorSearchQuery {
            vector: vec![1.0, 0.0],
            limit: 4,
        })
        .expect("equal-score search");
    assert_eq!(equal_score_hits[0].chunk_id, ChunkId::new(1));
    assert_eq!(equal_score_hits[1].chunk_id, ChunkId::new(2));
    assert!(
        !equal_score_hits
            .iter()
            .any(|hit| hit.chunk_id == ChunkId::new(4))
    );

    let zero_query_hits = index
        .search_similar(VectorSearchQuery {
            vector: vec![0.0, 0.0],
            limit: 10,
        })
        .expect("all-zero query search");
    assert!(
        zero_query_hits.is_empty(),
        "all-zero query must return no hits"
    );

    index
        .index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(7),
            vector: vec![0.0, 1.0],
            provenance: prov(),
        }])
        .expect("initial embedding");
    index
        .index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(7),
            vector: vec![1.0, 0.0],
            provenance: prov(),
        }])
        .expect("replacement embedding");
    let replacement_hits = index
        .search_similar(VectorSearchQuery {
            vector: vec![1.0, 0.0],
            limit: 10,
        })
        .expect("replacement search");
    let replaced = replacement_hits
        .iter()
        .filter(|hit| hit.chunk_id == ChunkId::new(7))
        .collect::<Vec<_>>();
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].score, 1.0);

    assert!(matches!(
        index.index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(9),
            vector: Vec::new(),
            provenance: prov(),
        }]),
        Err(PortError::InvalidInput { .. })
    ));
    assert!(matches!(
        index.search_similar(VectorSearchQuery {
            vector: vec![f32::NAN],
            limit: 1,
        }),
        Err(PortError::InvalidInput { .. })
    ));
    // Validate query vector before honoring limit=0.
    assert!(matches!(
        index.search_similar(VectorSearchQuery {
            vector: vec![f32::NAN],
            limit: 0,
        }),
        Err(PortError::InvalidInput { .. })
    ));
}

pub fn assert_parser_round_trip(parser: &impl Parser) {
    assert_eq!(parser.id(), "in-memory-parser");
    assert!(parser.supports(&FileMetadata {
        path: PathBuf::from("notes.md"),
        size: 5,
        extension: Some("md".to_string()),
    }));
    assert!(!parser.supports(&FileMetadata {
        path: PathBuf::from("archive.bin"),
        size: 5,
        extension: Some("bin".to_string()),
    }));

    let parsed = parser
        .parse(
            FileHandle {
                path: PathBuf::from("notes.md"),
                bytes: b"alpha".to_vec(),
            },
            ParseContext {
                artifact_id: ArtifactId::new(7),
            },
        )
        .expect("parse utf8 file");

    assert_eq!(parsed.artifact_id, ArtifactId::new(7));
    assert_eq!(parsed.chunks.len(), 1);
    assert_eq!(parsed.chunks[0].text, "alpha");
    assert_eq!(
        parsed.chunks[0].source_span,
        SourceSpan::TextSpan {
            start_line: 1,
            end_line: 1
        }
    );
    assert!(matches!(
        parser.parse(
            FileHandle {
                path: PathBuf::from("empty.md"),
                bytes: Vec::new(),
            },
            ParseContext {
                artifact_id: ArtifactId::new(8),
            },
        ),
        Err(PortError::InvalidInput { .. })
    ));
}

pub async fn assert_harness_adapter_round_trip(harness: &impl HarnessAdapter) {
    let capabilities = harness.capabilities().expect("capabilities");
    assert!(capabilities.read_enabled);
    assert!(capabilities.write_enabled);
    assert!(
        capabilities
            .command_classes
            .contains(&HarnessCommandClass::Shell)
    );

    let outcome = harness
        .execute(HarnessRequest {
            run_id: HarnessRunId::new(7),
            command: "echo ok".to_string(),
            working_directory: PathBuf::from("/tmp"),
            duration_budget: Duration::from_secs(1),
            class: HarnessCommandClass::Shell,
            readable_roots: vec![],
        })
        .await
        .expect("execute command");

    assert_eq!(outcome.run_id, HarnessRunId::new(7));
    assert_eq!(outcome.command, "echo ok");
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.stdout, b"executed echo ok".to_vec());

    assert!(matches!(
        harness
            .execute(HarnessRequest {
                run_id: HarnessRunId::new(8),
                command: " ".to_string(),
                working_directory: PathBuf::from("/tmp"),
                duration_budget: Duration::from_secs(1),
                class: HarnessCommandClass::Shell,
                readable_roots: vec![],
            })
            .await,
        Err(PortError::InvalidInput { .. })
    ));
}

pub fn assert_graph_index_contract(index: &impl GraphIndex) {
    let artifact_ep = RelationEndpoint::Artifact(ArtifactId::new(1));
    let card_ep = RelationEndpoint::Card(CardId::new(2));
    let claim_ep = RelationEndpoint::Claim(ClaimId::new(3));

    let mut rel3 = Relation {
        id: RelationId::new(3),
        source: artifact_ep,
        target: card_ep,
        kind: RelationKind::Contains,
        evidence_id: None,
        confidence_milli: 800,
    };
    let rel1 = Relation {
        id: RelationId::new(1),
        source: card_ep,
        target: claim_ep,
        kind: RelationKind::Supports,
        evidence_id: Some(EvidenceId::new(4)),
        confidence_milli: 900,
    };
    let rel2 = Relation {
        id: RelationId::new(2),
        source: artifact_ep,
        target: claim_ep,
        kind: RelationKind::Contradicts,
        evidence_id: None,
        confidence_milli: 500,
    };

    // Insert out of order
    index.insert_relation(rel3.clone()).expect("insert 3");
    index.insert_relation(rel1.clone()).expect("insert 1");
    index.insert_relation(rel2.clone()).expect("insert 2");

    // Replace 3
    rel3.confidence_milli = 950;
    index.insert_relation(rel3.clone()).expect("replace 3");

    // Query for artifact_ep, which is in rel3 (source) and rel2 (source)
    // Must be returned in order of RelationId: rel2 then rel3
    let artifact_rels = index
        .get_relations_for(artifact_ep)
        .expect("get relations for artifact");
    assert_eq!(artifact_rels.len(), 2);
    assert_eq!(artifact_rels[0], rel2);
    assert_eq!(artifact_rels[1], rel3);

    // Query for claim_ep, which is in rel1 (target) and rel2 (target)
    // Must be returned in order of RelationId: rel1 then rel2
    let claim_rels = index
        .get_relations_for(claim_ep)
        .expect("get relations for claim");
    assert_eq!(claim_rels.len(), 2);
    assert_eq!(claim_rels[0], rel1);
    assert_eq!(claim_rels[1], rel2);
}

pub fn assert_web_fetcher_contract(
    fetcher: &impl super::WebFetcher,
    valid_url: &str,
    valid_html: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let fetch_res = fetcher.fetch(valid_url)?;
    assert_eq!(fetch_res.url, valid_url, "URL must be preserved");
    assert_eq!(fetch_res.html, valid_html, "HTML must match");
    assert!(!fetch_res.html.is_empty(), "HTML should be non-empty");

    let empty_res = fetcher.fetch("");
    assert!(
        matches!(empty_res, Err(super::PortError::InvalidInput { .. })),
        "Empty URLs must map to PortError::InvalidInput, got {:?}",
        empty_res
    );

    Ok(())
}
