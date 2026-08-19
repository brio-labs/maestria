use maestria_domain::ChunkId;
use maestria_ports::{
    EmbeddingProvenance, PortError, ProviderDisclosure, RetentionPolicy, VectorEmbedding,
    VectorIndex, VectorSearchQuery,
};

use crate::encoding::to_port_error;
use crate::tests_support::search_budget;
use crate::vector_index::SqliteVectorIndex;

#[test]
fn round_trips_provenance() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let provenance = EmbeddingProvenance {
        content_hash: "hash_abcd".into(),
        identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 3)?,
        provider_id: "test-provider".into(),
        model: "test-model".into(),
        model_version: "model_v1".into(),
        disclosure: ProviderDisclosure {
            remote: true,
            retention: RetentionPolicy::ProviderDefined,
        },
    };

    index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(42),
        vector: vec![1.0, 0.5, 0.25],
        provenance: provenance.clone(),
    }])?;

    // Direct query to verify provenance storage, since the contract
    let connection = index.connection.lock().map_err(|_| {
        PortError::internal(
            "maestria vector sqlite test",
            "vector index lock poisoned".to_string(),
        )
    })?;
    let mut stmt = connection
        .prepare("SELECT content_hash, model_version, disclosure_remote, retention_policy FROM vector_embeddings WHERE chunk_id = 42")
        .map_err(to_port_error)?;
    let (hash, version, remote, retention): (String, String, i64, String) = stmt
        .query_row([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(to_port_error)?;

    assert_eq!(hash, provenance.content_hash);
    assert_eq!(version, provenance.model_version);
    assert_eq!(remote, 1);
    assert_eq!(retention, "provider_defined");
    Ok(())
}

#[test]
fn unchanged_embedding_does_not_update_projection() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let connection = index.connection.lock().map_err(|_| {
        PortError::internal(
            "maestria vector sqlite test",
            "vector index lock poisoned".to_string(),
        )
    })?;
    connection
        .execute_batch(
            "CREATE TABLE vector_write_audit (count INTEGER NOT NULL);
             INSERT INTO vector_write_audit (count) VALUES (0);
             CREATE TRIGGER vector_update_audit
             AFTER UPDATE ON vector_embeddings
             BEGIN
                 UPDATE vector_write_audit SET count = count + 1;
             END;",
        )
        .map_err(to_port_error)?;
    drop(connection);

    let embedding = VectorEmbedding {
        chunk_id: ChunkId::new(42),
        vector: vec![1.0, 0.5],
        provenance: EmbeddingProvenance {
            content_hash: "hash".to_string(),
            identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 2)?,
            provider_id: "test-provider".into(),
            model: "test-model".into(),
            model_version: "model-v1".to_string(),
            disclosure: ProviderDisclosure {
                remote: false,
                retention: RetentionPolicy::NoRetention,
            },
        },
    };
    index.index_embeddings(vec![embedding.clone()])?;
    index.index_embeddings(vec![embedding])?;

    let connection = index.connection.lock().map_err(|_| {
        PortError::internal(
            "maestria vector sqlite test",
            "vector index lock poisoned".to_string(),
        )
    })?;
    let writes: i64 = connection
        .query_row("SELECT count FROM vector_write_audit", [], |row| row.get(0))
        .map_err(to_port_error)?;
    assert_eq!(writes, 0);
    Ok(())
}

#[test]
fn reopen_persistence_and_mismatch_rejection() -> Result<(), PortError> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| {
            PortError::internal(
                "maestria vector sqlite test",
                format!("read test timestamp: {error}"),
            )
        })?;
    let db_path = std::env::temp_dir().join(format!("test_vec_{}.db", timestamp));

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

    {
        let index = SqliteVectorIndex::open(&db_path)?;
        index.index_embeddings(vec![VectorEmbedding {
            chunk_id: ChunkId::new(1),
            vector: vec![1.0, 0.0],
            provenance: prov.clone(),
        }])?;
    }

    // Reopen and check persistence
    let index = SqliteVectorIndex::open(&db_path)?;
    let hits = index.search_similar(VectorSearchQuery {
        identity: Some(prov.identity.clone()),
        vector: vec![1.0, 0.0],
        limit: 1,
        provider_id: None,
        model: None,
        model_version: None,
        execution_budget: search_budget(1)?,
    })?;
    assert_eq!(hits.hits.len(), 1);

    // Mismatch rejection
    let mut bad_ident = prov.identity.clone();
    bad_ident.fingerprint.dimensions = 3;

    let res = index.search_similar(VectorSearchQuery {
        identity: Some(bad_ident),
        vector: vec![1.0, 0.0],
        limit: 1,
        provider_id: None,
        model: None,
        model_version: None,
        execution_budget: search_budget(1)?,
    });

    assert!(
        matches!(res, Err(PortError::InvalidInputContext { .. })),
        "Expected InvalidInput, got {:?}",
        res
    );
    let _ = std::fs::remove_file(db_path);
    Ok(())
}

#[test]
fn rebuild_replaces_and_deletes_stale_rows() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let prov = EmbeddingProvenance {
        content_hash: "hash1".into(),
        identity: maestria_ports::contract_tests::fixture_embedding_identity("test-model", 2)?,
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
            vector: vec![1.0, 0.0],
            provenance: prov.clone(),
        },
        VectorEmbedding {
            chunk_id: ChunkId::new(2),
            vector: vec![0.0, 1.0],
            provenance: prov.clone(),
        },
    ])?;

    // Rebuild with only chunk 2 (modified) and chunk 3 (new)
    let mut prov2 = prov.clone();
    prov2.content_hash = "hash2".into();
    index.rebuild(vec![
        VectorEmbedding {
            chunk_id: ChunkId::new(2),
            vector: vec![0.5, 0.5],
            provenance: prov2.clone(),
        },
        VectorEmbedding {
            chunk_id: ChunkId::new(3),
            vector: vec![1.0, 1.0],
            provenance: prov2.clone(),
        },
    ])?;

    // Check that chunk 1 is gone
    let hits_1 = index.search_similar_filtered(
        VectorSearchQuery {
            identity: None,
            vector: vec![1.0, 0.0],
            limit: 10,
            provider_id: None,
            model: None,
            model_version: None,
            execution_budget: search_budget(10)?,
        },
        &|id| Ok(id.value() == 1),
    )?;
    assert!(hits_1.hits.is_empty(), "chunk 1 should be deleted");

    // Check that chunk 2 is updated (should match 0.5, 0.5 exactly)
    let hits_2 = index.search_similar_filtered(
        VectorSearchQuery {
            identity: None,
            vector: vec![0.5, 0.5],
            limit: 10,
            provider_id: None,
            model: None,
            model_version: None,
            execution_budget: search_budget(10)?,
        },
        &|id| Ok(id.value() == 2),
    )?;
    assert_eq!(hits_2.hits.len(), 1);
    assert_eq!(hits_2.hits[0].score, 1.0);

    // Check that chunk 3 is inserted
    let hits_3 = index.search_similar_filtered(
        VectorSearchQuery {
            identity: None,
            vector: vec![1.0, 1.0],
            limit: 10,
            provider_id: None,
            model: None,
            model_version: None,
            execution_budget: search_budget(10)?,
        },
        &|id| Ok(id.value() == 3),
    )?;
    assert_eq!(hits_3.hits.len(), 1);

    Ok(())
}
