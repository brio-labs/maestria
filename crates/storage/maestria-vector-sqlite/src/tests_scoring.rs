use maestria_domain::ChunkId;
use maestria_ports::{
    EmbeddingProvenance, PortError, ProviderDisclosure, RetentionPolicy, VectorEmbedding,
    VectorIndex, VectorSearchQuery,
};

use crate::tests_support::search_budget;
use crate::vector_index::SqliteVectorIndex;

#[test]
fn prevents_nan_scores_from_overflow() -> Result<(), PortError> {
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

    // Vectors that might cause f32 overflow when accumulating sum of squares
    // e.g. a vector with values near sqrt(f32::MAX) ~= 1.8e19
    let huge_val = 1.0e19_f32;
    index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(1),
        vector: vec![huge_val, huge_val],
        provenance: prov,
    }])?;

    let hits = index.search_similar(VectorSearchQuery {
        identity: None,
        vector: vec![huge_val, huge_val],
        limit: 1,
        provider_id: None,
        model: None,
        model_version: None,
        execution_budget: search_budget(1)?,
    })?;

    assert_eq!(hits.hits.len(), 1);
    assert!(
        hits.hits[0].score.is_finite(),
        "Score should be finite despite huge values"
    );
    assert_eq!(hits.hits[0].score, 1.0); // Exact match is 1.0
    Ok(())
}
