use maestria_domain::ChunkId;
use maestria_ports::{
    EmbeddingProvenance, PortError, ProviderDisclosure, RetentionPolicy, VectorEmbedding,
    VectorIndex, VectorSearchQuery, contract_tests::assert_vector_index_contract,
};

use crate::tests_support::search_budget;
use crate::vector_index::SqliteVectorIndex;

#[test]
fn rejects_empty_vector_on_index() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let result = index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(1),
        vector: vec![],
        provenance: EmbeddingProvenance {
            content_hash: "hash".into(),
            identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 0)?,
            provider_id: "test-provider".into(),
            model: "test-model".into(),
            model_version: "v1".into(),
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        },
    }]);
    assert!(
        matches!(result, Err(PortError::InvalidInputContext { .. })),
        "expected InvalidInput for empty vector, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_missing_provenance_on_index() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let result = index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(1),
        vector: vec![1.0, 0.5],
        provenance: EmbeddingProvenance {
            content_hash: "".into(),
            identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 2)?,
            provider_id: "test-provider".into(),
            model: "test-model".into(),
            model_version: "v1".into(),
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        },
    }]);
    assert!(
        matches!(result, Err(PortError::InvalidInputContext { .. })),
        "expected InvalidInput for missing provenance, got {result:?}"
    );
    Ok(())
}

#[test]
fn rejects_dimension_mismatch_on_index() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let result = index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(1),
        vector: vec![1.0, 0.5, 0.25],
        provenance: EmbeddingProvenance {
            content_hash: "hash".into(),
            identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 2)?,
            provider_id: "test-provider".into(),
            model: "test-model".into(),
            model_version: "v1".into(),
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        },
    }]);
    assert!(
        matches!(result, Err(PortError::InvalidInputContext { .. })),
        "expected InvalidInput for dimension mismatch, got {result:?}"
    );
    Ok(())
}

#[test]
fn counts_persisted_embeddings() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    assert_eq!(index.embedding_row_count()?, 0);
    let identity = maestria_ports::contract_tests::fixture_embedding_identity("test-model", 2)?;
    let provenance = |chunk_id: u64| EmbeddingProvenance {
        content_hash: format!("hash-{chunk_id}"),
        identity: identity.clone(),
        provider_id: "test-provider".into(),
        model: "test-model".into(),
        model_version: "v1".into(),
        disclosure: ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        },
    };
    index.index_embeddings(vec![
        VectorEmbedding {
            chunk_id: ChunkId::new(1),
            vector: vec![1.0, 0.5],
            provenance: provenance(1),
        },
        VectorEmbedding {
            chunk_id: ChunkId::new(2),
            vector: vec![0.5, 1.0],
            provenance: provenance(2),
        },
    ])?;
    assert_eq!(index.embedding_row_count()?, 2);
    Ok(())
}

#[test]
fn search_returns_empty_for_zero_norm_vector() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let prov = EmbeddingProvenance {
        content_hash: "hash".into(),
        identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 2)?,
        provider_id: "test-provider".into(),
        model: "test-model".into(),
        model_version: "v1".into(),
        disclosure: ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        },
    };
    index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(1),
        vector: vec![1.0, 0.0],
        provenance: prov.clone(),
    }])?;
    let hits = index.search_similar(VectorSearchQuery {
        identity: None,
        vector: vec![0.0, 0.0],
        limit: 10,
        provider_id: None,
        model: None,
        model_version: None,
        execution_budget: search_budget(10)?,
    })?;
    assert!(
        hits.hits.is_empty(),
        "expected no hits for zero-norm query vector"
    );
    Ok(())
}

#[test]
fn satisfies_shared_vector_index_contract() -> Result<(), Box<dyn std::error::Error>> {
    let index = SqliteVectorIndex::in_memory()?;
    assert_vector_index_contract(&index)?;
    Ok(())
}
