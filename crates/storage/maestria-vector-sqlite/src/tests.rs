use maestria_domain::ChunkId;
use maestria_ports::{
    EmbeddingProvenance, PortError, VectorEmbedding, VectorIndex, VectorSearchQuery,
    contract_tests::assert_vector_index_contract,
};
use rusqlite::Connection;

use super::{SqliteVectorIndex, to_port_error};
use crate::schema::SCHEMA_VERSION;
#[test]
fn satisfies_shared_vector_index_contract() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    assert_vector_index_contract(&index);
    Ok(())
}

#[test]
fn round_trips_provenance() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let provenance = EmbeddingProvenance {
        content_hash: "hash_abcd".into(),
        provider_id: "test-provider".into(),
        model: "test-model".into(),
        model_version: "model_v1".into(),
    };

    index.index_embeddings(vec![VectorEmbedding {
        chunk_id: ChunkId::new(42),
        vector: vec![1.0, 0.5, 0.25],
        provenance: provenance.clone(),
    }])?;

    // Direct query to verify provenance storage, since the contract
    let connection = index.connection.lock().map_err(|_| PortError::Internal {
        message: "vector index lock poisoned".to_string(),
    })?;
    let mut stmt = connection
        .prepare("SELECT content_hash, model_version FROM vector_embeddings WHERE chunk_id = 42")
        .map_err(to_port_error)?;
    let (hash, version): (String, String) = stmt
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(to_port_error)?;

    assert_eq!(hash, provenance.content_hash);
    assert_eq!(version, provenance.model_version);
    Ok(())
}

#[test]
fn unchanged_embedding_does_not_update_projection() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let connection = index.connection.lock().map_err(|_| PortError::Internal {
        message: "vector index lock poisoned".to_string(),
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
            provider_id: "test-provider".into(),
            model: "test-model".into(),
            model_version: "model-v1".to_string(),
        },
    };
    index.index_embeddings(vec![embedding.clone()])?;
    index.index_embeddings(vec![embedding])?;

    let connection = index.connection.lock().map_err(|_| PortError::Internal {
        message: "vector index lock poisoned".to_string(),
    })?;
    let writes: i64 = connection
        .query_row("SELECT count FROM vector_write_audit", [], |row| row.get(0))
        .map_err(to_port_error)?;
    assert_eq!(writes, 0);
    Ok(())
}

#[test]
fn rejects_unsupported_schema_version() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    conn.execute_batch(
        "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
         INSERT INTO vector_projection_schema (id, version) VALUES (1, 999);",
    )
    .map_err(to_port_error)?;

    match super::migrate(&mut conn) {
        Err(PortError::Internal { message }) => {
            assert!(message.contains("unsupported vector projection schema version 999"));
        }
        Err(_) => {
            return Err(PortError::Internal {
                message: "Expected unsupported version error, got different error".to_string(),
            });
        }
        Ok(_) => {
            return Err(PortError::Internal {
                message: "Expected error but got Ok".to_string(),
            });
        }
    }
    Ok(())
}
#[test]
fn rejects_zero_schema_version() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    conn.execute_batch(
        "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
         INSERT INTO vector_projection_schema (id, version) VALUES (1, 0);",
    )
    .map_err(to_port_error)?;

    match super::migrate(&mut conn) {
        Err(PortError::Internal { message }) => {
            assert!(message.contains("unsupported vector projection schema version 0"));
        }
        Err(_) => {
            return Err(PortError::Internal {
                message: "Expected unsupported version error, got different error".to_string(),
            });
        }
        Ok(_) => {
            return Err(PortError::Internal {
                message: "Expected error but got Ok".to_string(),
            });
        }
    }
    Ok(())
}

#[test]
fn migrates_version_1_schema_to_current() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    conn.execute_batch(
        "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
         INSERT INTO vector_projection_schema (id, version) VALUES (1, 1);
         CREATE TABLE vector_embeddings (
             chunk_id INTEGER PRIMARY KEY NOT NULL,
             dimension INTEGER NOT NULL,
             embedding BLOB NOT NULL
         );",
    )
    .map_err(to_port_error)?;

    super::migrate(&mut conn)?;

    let v: i64 = conn
        .query_row(
            "SELECT version FROM vector_projection_schema WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .map_err(to_port_error)?;
    assert_eq!(v, SCHEMA_VERSION);

    // Verify new columns exist by doing a dummy insert
    conn.execute("INSERT INTO vector_embeddings (chunk_id, dimension, embedding, content_hash, provider_id, model, model_version) VALUES (1, 1, X'00', 'a', 'provider', 'model', 'b')", []).map_err(to_port_error)?;
    Ok(())
}

#[test]
fn sqlite_vec_detection_verifies_virtual_table() -> Result<(), PortError> {
    let conn = Connection::open_in_memory().map_err(to_port_error)?;
    // Create a regular table named vec_docs with spoofed comment
    conn.execute("CREATE TABLE vec_docs (id INTEGER /* USING VEC0 */)", [])
        .map_err(to_port_error)?;

    assert!(!super::sqlite_vec_available(&conn)?);

    Ok(())
}

#[test]
fn prevents_nan_scores_from_overflow() -> Result<(), PortError> {
    let index = SqliteVectorIndex::in_memory()?;
    let prov = EmbeddingProvenance {
        content_hash: "hash".into(),
        provider_id: "test-provider".into(),
        model: "test-model".into(),
        model_version: "v1".into(),
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
        vector: vec![huge_val, huge_val],
        limit: 1,
        provider_id: None,
        model: None,
        model_version: None,
    })?;

    assert_eq!(hits.len(), 1);
    assert!(
        hits[0].score.is_finite(),
        "Score should be finite despite huge values"
    );
    assert_eq!(hits[0].score, 1.0); // Exact match is 1.0
    Ok(())
}
