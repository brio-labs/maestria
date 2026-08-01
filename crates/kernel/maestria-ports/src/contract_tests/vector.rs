use super::*;
use maestria_domain::ChunkId;

use super::fixtures::*;

pub fn assert_vector_index_contract(
    index: &impl VectorIndex,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity = EmbeddingIdentity::legacy("test-model", 2)?;
    let prov = || EmbeddingProvenance {
        content_hash: "abcd123".into(),
        identity: identity.clone(),
        provider_id: "test-provider".into(),
        model: "test-model".into(),
        model_version: "test-v1".into(),
        disclosure: ProviderDisclosure {
            remote: false,
            retention: RetentionPolicy::NoRetention,
        },
    };
    let embedding = |chunk_id, vector| VectorEmbedding {
        chunk_id: ChunkId::new(chunk_id),
        vector,
        provenance: prov(),
    };
    index.index_embeddings(vec![
        embedding(2, vec![1.0, 0.0]),
        embedding(1, vec![1.0, 0.0]),
        embedding(3, vec![0.0, 1.0]),
    ])?;
    assert!(matches!(
        index.index_embeddings(vec![embedding(4, vec![1.0, 0.0, 0.0])]),
        Err(error) if error.is_invalid_input()
    ));
    let equal_score_hits = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 4,
        execution_budget: search_budget(4)?,
        ..Default::default()
    })?;
    assert_eq!(equal_score_hits.execution.budget.max_results(), 4);
    assert_eq!(equal_score_hits.hits[0].chunk_id, ChunkId::new(1));
    assert_eq!(equal_score_hits.hits[1].chunk_id, ChunkId::new(2));
    assert!(
        !equal_score_hits
            .hits
            .iter()
            .any(|hit| hit.chunk_id == ChunkId::new(4))
    );
    verify_vector_identity_filter(index)?;

    let zero_query_hits = index.search_similar(VectorSearchQuery {
        vector: vec![0.0, 0.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    assert!(
        zero_query_hits.hits.is_empty(),
        "all-zero query must return no hits"
    );

    index.index_embeddings(vec![embedding(7, vec![0.0, 1.0])])?;
    index.index_embeddings(vec![embedding(7, vec![1.0, 0.0])])?;
    let replacement_hits = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    let replaced = replacement_hits
        .hits
        .iter()
        .filter(|hit| hit.chunk_id == ChunkId::new(7))
        .collect::<Vec<_>>();
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].score, 1.0);
    verify_vector_validation(index, &prov)?;
    verify_vector_lifecycle(index, prov)?;
    Ok(())
}

fn verify_vector_identity_filter(
    index: &impl VectorIndex,
) -> Result<(), Box<dyn std::error::Error>> {
    let mismatched_identity_hits = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 4,
        execution_budget: search_budget(4)?,
        provider_id: Some("other-provider".into()),
        model: Some("other-model".into()),
        model_version: Some("other-version".into()),
        identity: None,
    })?;
    assert!(
        mismatched_identity_hits.hits.is_empty(),
        "provider/model/version identity must filter incompatible rows"
    );
    Ok(())
}

fn verify_vector_validation(
    index: &impl VectorIndex,
    prov: &impl Fn() -> EmbeddingProvenance,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(matches!(
        index.index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(9),
            vector: Vec::new(),
            provenance: prov(),
        }]),
        Err(PortError::InvalidInputContext { .. })
    ));
    assert!(matches!(
        index.search_similar(VectorSearchQuery {
            vector: vec![f32::NAN],
            limit: 1,
            execution_budget: search_budget(1)?,
            provider_id: None,
            model: None,
            model_version: None,
            identity: None,
        }),
        Err(PortError::InvalidInputContext { .. })
    ));
    assert!(matches!(
        index.search_similar(VectorSearchQuery {
            vector: vec![f32::NAN],
            limit: 1,
            execution_budget: search_budget(1)?,
            provider_id: None,
            model: None,
            model_version: None,
            identity: None,
        }),
        Err(PortError::InvalidInputContext { .. })
    ));
    Ok(())
}

fn verify_vector_lifecycle(
    index: &impl VectorIndex,
    prov: impl Fn() -> EmbeddingProvenance,
) -> Result<(), Box<dyn std::error::Error>> {
    let embedding = |chunk_id, vector| VectorEmbedding {
        chunk_id: ChunkId::new(chunk_id),
        vector,
        provenance: prov(),
    };
    index.clear()?;
    let hits_after_clear = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    assert!(
        hits_after_clear.hits.is_empty(),
        "index must be empty after clear"
    );
    index.clear()?;
    index.rebuild(vec![
        embedding(10, vec![0.0, 1.0]),
        embedding(11, vec![1.0, 0.0]),
    ])?;
    let hits_after_rebuild = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    assert_eq!(
        hits_after_rebuild.hits.len(),
        2,
        "must have exactly two hits after rebuild"
    );
    assert_eq!(hits_after_rebuild.hits[0].chunk_id, ChunkId::new(11));
    index.delete_chunks(&[ChunkId::new(10)])?;
    let hits_after_delete = index.search_similar(VectorSearchQuery {
        vector: vec![0.0, 1.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    assert_eq!(
        hits_after_delete.hits.len(),
        1,
        "must have one hit remaining after delete"
    );
    assert_eq!(
        hits_after_delete.hits[0].chunk_id,
        ChunkId::new(11),
        "only chunk 11 should remain"
    );
    index.delete_chunks(&[ChunkId::new(10), ChunkId::new(999)])?;
    let hits_after_idempotent_delete = index.search_similar(VectorSearchQuery {
        vector: vec![1.0, 0.0],
        limit: 10,
        execution_budget: search_budget(10)?,
        ..Default::default()
    })?;
    assert_eq!(
        hits_after_idempotent_delete.hits.len(),
        1,
        "must still have one hit remaining"
    );
    assert_eq!(
        hits_after_idempotent_delete.hits[0].chunk_id,
        ChunkId::new(11)
    );
    Ok(())
}
