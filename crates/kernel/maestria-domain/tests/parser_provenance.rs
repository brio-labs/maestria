use maestria_domain::*;

fn root() -> StructureNode {
    StructureNode {
        id: StructureNodeId::new(10),
        parent_id: None,
        sibling_id: None,
        node_type: StructureNodeType::Document,
        source_range: ContentRange { start: 0, end: 5 },
        page: None,
        section_path: vec!["document".to_owned()],
        parser_generation: "parser-v1".to_owned(),
        schema_generation: "schema-v1".to_owned(),
        language: Some("markdown".to_owned()),
    }
}

fn parsed_result(status: ParseStatus) -> ParserResult {
    let node = root();
    ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(2),
        content_hash: ContentHash::new("sha256:".to_owned() + &"a".repeat(64))
            .expect("valid test hash"),
        status,
        tree_root_id: (status == ParseStatus::Parsed).then_some(node.id),
        tree_nodes: (status == ParseStatus::Parsed)
            .then_some(node)
            .into_iter()
            .collect(),
        chunks: if status == ParseStatus::Parsed {
            vec![RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                source_span: SourceSpan::TextSpan {
                    start_line: 2,
                    end_line: 4,
                },
                representations: vec![
                    ParsedRepresentation {
                        kind: RepresentationKind::Raw,
                        content: "raw\n".to_owned(),
                    },
                    ParsedRepresentation {
                        kind: RepresentationKind::Retrieval,
                        content: "retrieval".to_owned(),
                    },
                ],
                order: 0,
                text: "retrieval".to_owned(),
            }]
        } else {
            Vec::new()
        },
        cards: if status == ParseStatus::Parsed {
            vec![CreateCardInput {
                card_id: CardId::new(12),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                source_span: SourceSpan::TextSpan {
                    start_line: 1,
                    end_line: 4,
                },
                title: "Document".to_owned(),
                body: "A source-backed summary".to_owned(),
            }]
        } else {
            Vec::new()
        },
    }
}

#[test]
fn parsed_provenance_survives_event_replay() {
    let inputs = vec![
        DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "notes.md".to_owned(),
            source_path: "notes.md".to_owned(),
            source_bytes: b"# notes".to_vec(),
            content_hash: "sha256:source".to_owned(),
        }),
        DomainInput::ParserCompleted(parsed_result(ParseStatus::Parsed)),
    ];

    let (state, events, _) = replay_inputs(&inputs).expect("inputs replay");
    let rebuilt = replay_events(&events).expect("event replay");

    assert_eq!(state, rebuilt);
    assert_eq!(state.chunks[&ChunkId::new(11)].representations.len(), 2);
    assert_eq!(
        state.chunks[&ChunkId::new(11)].source_span,
        SourceSpan::TextSpan {
            start_line: 2,
            end_line: 4
        }
    );
    assert_eq!(
        state.cards[&CardId::new(12)].node_id,
        StructureNodeId::new(10)
    );
}

#[test]
fn non_parsed_status_is_replayed_without_index_work() {
    let inputs = vec![
        DomainInput::ArtifactDetected(ArtifactDetected {
            artifact_id: ArtifactId::new(1),
            title: "scan.pdf".to_owned(),
            source_path: "scan.pdf".to_owned(),
            source_bytes: b"opaque".to_vec(),
            content_hash: "sha256:source".to_owned(),
        }),
        DomainInput::ParserCompleted(parsed_result(ParseStatus::NeedsOcr)),
    ];

    let (state, events, effects) = replay_inputs(&inputs).expect("inputs replay");
    let rebuilt = replay_events(&events).expect("event replay");

    assert_eq!(state, rebuilt);
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].parse_status,
        Some(ParseStatus::NeedsOcr)
    );
    assert_eq!(
        state.artifacts[&ArtifactId::new(1)].index_status,
        IndexStatus::Unindexed
    );
    assert!(state.chunks.is_empty());
    assert!(state.cards.is_empty());
    assert!(state.pending_full_text.is_empty());
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        MaestriaEffect::IndexFullText(_) | MaestriaEffect::IndexVector(_)
    )));
}
