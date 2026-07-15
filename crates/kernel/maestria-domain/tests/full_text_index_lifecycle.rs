use maestria_domain::*;
#[path = "common/fixtures.rs"]
mod fixtures;

#[test]
fn start_full_text_index_emits_for_pending_chunks() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                order: 0,
                text: "chunk a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(11),
                order: 1,
                text: "chunk b".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;

    let output = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }))?;
    let index_effects: Vec<_> = output
        .effects
        .iter()
        .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
        .collect();
    assert_eq!(index_effects.len(), 2);
    assert!(
        matches!(&index_effects[0], MaestriaEffect::IndexFullText(req) if req.chunk_id == ChunkId::new(10))
    );
    assert!(
        matches!(&index_effects[1], MaestriaEffect::IndexFullText(req) if req.chunk_id == ChunkId::new(11))
    );
    assert!(state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));
    Ok(())
}

#[test]
fn start_full_text_index_only_pending_chunks_on_retry() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![
            RegisterChunkInput {
                chunk_id: ChunkId::new(10),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(10),
                order: 0,
                text: "chunk a".to_string(),
            },
            RegisterChunkInput {
                chunk_id: ChunkId::new(11),
                artifact_id: ArtifactId::new(1),
                node_id: StructureNodeId::new(11),
                order: 1,
                text: "chunk b".to_string(),
            },
        ],
        cards: Vec::new(),
    }))?;
    state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }))?;
    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    assert!(!state.pending_full_text.contains(&ChunkId::new(10)));
    assert!(state.pending_full_text.contains(&ChunkId::new(11)));

    let output = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }))?;
    let index_effects: Vec<_> = output
        .effects
        .iter()
        .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
        .collect();
    assert_eq!(index_effects.len(), 1);
    assert!(
        matches!(&index_effects[0], MaestriaEffect::IndexFullText(req) if req.chunk_id == ChunkId::new(11))
    );
    Ok(())
}

#[test]
fn start_full_text_index_duplicate_is_idempotent() -> Result<(), DomainError> {
    let mut state = KernelState::new();
    state.apply_input(DomainInput::ArtifactDetected(ArtifactDetected {
        artifact_id: ArtifactId::new(1),
        title: "Doc".to_string(),
        source_path: String::new(),
        source_bytes: vec![1, 2, 3],
        content_hash: "sha256:abc".to_string(),
    }))?;
    state.apply_input(DomainInput::ParserCompleted(ParserResult {
        artifact_id: ArtifactId::new(1),
        artifact_version_id: ArtifactVersionId::new(1),
        content_hash: fixtures::test_content_hash(),
        tree_root_id: StructureNodeId::new(10),
        tree_nodes: vec![fixtures::tree_root_node(StructureNodeId::new(10))],
        chunks: vec![RegisterChunkInput {
            chunk_id: ChunkId::new(10),
            artifact_id: ArtifactId::new(1),
            node_id: StructureNodeId::new(10),
            order: 0,
            text: "chunk a".to_string(),
        }],
        cards: Vec::new(),
    }))?;

    let output1 = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }))?;
    assert_eq!(
        output1
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
            .count(),
        1
    );
    let output2 = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }))?;
    assert_eq!(
        output2
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
            .count(),
        1
    );

    state.apply_input(DomainInput::FullTextIndexCompleted(
        FullTextIndexCompleted {
            artifact_id: ArtifactId::new(1),
            chunk_id: ChunkId::new(10),
        },
    ))?;
    let output3 = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }))?;
    assert_eq!(
        output3
            .effects
            .iter()
            .filter(|e| matches!(e, MaestriaEffect::IndexFullText(_)))
            .count(),
        0
    );
    Ok(())
}

#[test]
fn start_full_text_index_rejects_missing_artifact() {
    let mut state = KernelState::new();
    let result = state.apply_input(DomainInput::StartFullTextIndex(StartFullTextIndex {
        artifact_id: ArtifactId::new(1),
    }));
    assert!(matches!(
        result,
        Err(DomainError::MissingArtifact { id: ArtifactId(1) })
    ));
}
