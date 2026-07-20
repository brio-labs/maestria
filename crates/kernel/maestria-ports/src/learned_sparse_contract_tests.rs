use maestria_domain::{ContentHash, CorpusSnapshotId, IndexGenerationId, RepresentationName};

use crate::{
    ChunkId, LearnedSparseIndex, LearnedSparseProvider, PortError, SPARSE_REPRESENTATION_V1,
    SparseDocument, SparseFingerprint, SparseIdentity, SparseInputKind, SparseSearchQuery,
};

pub fn fixture_sparse_identity() -> Result<SparseIdentity, PortError> {
    let hash = |digit: char| {
        maestria_domain::ContentHash::new(format!("sha256:{}", digit.to_string().repeat(64)))
            .map_err(|error| PortError::InvalidInput {
                message: format!("create sparse fixture hash: {error}"),
            })
    };
    Ok(SparseIdentity {
        generation_id: IndexGenerationId::new(7),
        corpus_snapshot: CorpusSnapshotId::new(11),
        representation: RepresentationName::new(SPARSE_REPRESENTATION_V1),
        fingerprint: SparseFingerprint {
            provider: "fixture-local".to_string(),
            model: "fixture-sparse".to_string(),
            revision: "v1".to_string(),
            artifact_hash: hash('1')?,
            tokenizer_hash: hash('2')?,
            vocabulary_hash: hash('3')?,
            vocabulary_size: 65_536,
            term_namespace: "fixture-vocabulary-v1".to_string(),
            query_template_hash: "sha256:query-template".to_string(),
            document_template_hash: "sha256:document-template".to_string(),
            preprocessing_version: "fixture-preprocess-v1".to_string(),
            weighting_version: "fixture-log-frequency-v1".to_string(),
            quantization: "f32".to_string(),
            pruning_threshold: 0.0,
            max_terms: 128,
        },
    })
}

pub fn assert_learned_sparse_provider_contract(
    provider: &impl LearnedSparseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity = provider
        .identity()
        .ok_or("learned sparse provider identity is unavailable")?;
    identity.validate()?;
    let disclosure = provider
        .disclosure()
        .ok_or("learned sparse provider disclosure is unavailable")?;
    assert!(!disclosure.remote);
    assert_eq!(disclosure.retention, crate::RetentionPolicy::NoRetention);

    let document = provider.encode(
        "alpha alpha beta",
        SparseInputKind::Document,
        identity.clone(),
    )?;
    let repeated = provider.encode(
        "alpha alpha beta",
        SparseInputKind::Document,
        identity.clone(),
    )?;
    assert_eq!(document, repeated);
    assert_eq!(document.identity(), &identity);
    assert!(!document.terms().is_empty());

    let query = provider.encode("alpha", SparseInputKind::Query, identity.clone())?;
    assert_eq!(query.identity(), &identity);
    assert!(matches!(
        provider.encode("", SparseInputKind::Query, identity.clone()),
        Err(PortError::InvalidInput { .. })
    ));

    let mut incompatible = identity;
    incompatible.generation_id = IndexGenerationId::new(99);
    assert!(matches!(
        provider.encode("alpha", SparseInputKind::Query, incompatible),
        Err(PortError::InvalidInput { .. })
    ));
    Ok(())
}

pub fn assert_learned_sparse_index_contract(
    index: &impl LearnedSparseIndex,
    provider: &impl LearnedSparseProvider,
) -> Result<(), Box<dyn std::error::Error>> {
    let identity = provider
        .identity()
        .ok_or("learned sparse provider identity is unavailable")?;
    let content_hash = ContentHash::new(format!("sha256:{}", "4".repeat(64)))?;
    let document = |chunk_id, text: &str| -> Result<SparseDocument, PortError> {
        Ok(SparseDocument {
            chunk_id: ChunkId::new(chunk_id),
            content_hash: content_hash.clone(),
            vector: provider.encode(text, SparseInputKind::Document, identity.clone())?,
        })
    };

    index.index_documents(vec![
        document(2, "alpha beta")?,
        document(1, "alpha beta")?,
        document(3, "gamma delta")?,
    ])?;
    let query_vector = provider.encode("alpha", SparseInputKind::Query, identity.clone())?;
    let query = SparseSearchQuery {
        vector: query_vector,
        limit: 10,
        max_contributions: 8,
    };
    let hits = index.search(query.clone())?;
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].chunk_id, ChunkId::new(1));
    assert_eq!(hits[1].chunk_id, ChunkId::new(2));
    assert!(hits.iter().all(|hit| hit.score_micros > 0));
    assert!(hits.iter().all(|hit| !hit.contributions.is_empty()));

    let filtered = index.search_filtered(query.clone(), &|chunk_id| chunk_id == ChunkId::new(2))?;
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].chunk_id, ChunkId::new(2));

    index.index_documents(vec![document(2, "gamma delta")?])?;
    let replaced = index.search(query.clone())?;
    assert_eq!(replaced.len(), 1);
    assert_eq!(replaced[0].chunk_id, ChunkId::new(1));

    index.delete_chunks(&[ChunkId::new(1)])?;
    assert!(index.search(query.clone())?.is_empty());

    index.rebuild(vec![document(5, "alpha epsilon")?])?;
    let rebuilt = index.search(query)?;
    assert_eq!(rebuilt.len(), 1);
    assert_eq!(rebuilt[0].chunk_id, ChunkId::new(5));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InMemoryLearnedSparseIndex, InMemoryLearnedSparseProvider};

    #[test]
    fn in_memory_provider_obeys_sparse_contract() -> Result<(), Box<dyn std::error::Error>> {
        let provider = InMemoryLearnedSparseProvider::new(fixture_sparse_identity()?)?;
        assert_learned_sparse_provider_contract(&provider)
    }

    #[test]
    fn in_memory_index_obeys_sparse_contract() -> Result<(), Box<dyn std::error::Error>> {
        let provider = InMemoryLearnedSparseProvider::new(fixture_sparse_identity()?)?;
        assert_learned_sparse_index_contract(&InMemoryLearnedSparseIndex::new(), &provider)
    }
}
