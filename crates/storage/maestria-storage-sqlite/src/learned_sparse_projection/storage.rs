use maestria_domain::{ContentHash, IndexLifecycle};
use maestria_ports::{PortError, SparseDocument, SparseIdentity, SparseTermWeight, SparseVector};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::{SqliteStore, sqlite_store::to_port_error};

const MAX_SPARSE_VECTOR_BYTES: usize = 1_048_576;

#[derive(Debug, Clone)]
pub(super) struct StoredDocument {
    pub(super) chunk_id: maestria_domain::ChunkId,
    pub(super) vector: SparseVector,
    pub(super) encoded_bytes: u64,
}

pub(super) struct LoadedDocuments {
    pub(super) documents: Vec<StoredDocument>,
    pub(super) candidate_limit_reached: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredVector {
    identity: SparseIdentity,
    terms: Vec<StoredTerm>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredTerm {
    term_id: u32,
    weight: f32,
}

pub(super) fn identity_json(identity: &SparseIdentity) -> Result<String, PortError> {
    serde_json::to_string(identity).map_err(json_error)
}

pub(super) fn ensure_generation(
    store: &SqliteStore,
    identity: &SparseIdentity,
) -> Result<(), PortError> {
    let identity_json = identity_json(identity)?;
    let namespace_json = serde_json::to_string(&identity.namespace).map_err(json_error)?;
    let fingerprint_json = serde_json::to_string(&identity.fingerprint).map_err(json_error)?;
    let lifecycle = lifecycle_json(IndexLifecycle::Building)?;
    let connection = store.lock()?;
    let existing: Option<String> = connection
        .query_row(
            "SELECT identity_json FROM learned_sparse_projections WHERE generation_id = ?1",
            params![to_i64(identity.generation_id.value())?],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    if let Some(existing) = existing
        && existing != identity_json
    {
        return Err(PortError::Conflict {
            message: "sparse generation ID is bound to another identity".to_string(),
        });
    }
    connection
        .execute(
            "INSERT OR IGNORE INTO learned_sparse_projections
             (identity_json, generation_id, corpus_snapshot, namespace_json, fingerprint_json, lifecycle)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                identity_json,
                to_i64(identity.generation_id.value())?,
                to_i64(identity.corpus_snapshot.value())?,
                namespace_json,
                fingerprint_json,
                lifecycle,
            ],
        )
        .map_err(to_port_error)?;
    drop(connection);
    validate_generation(store, identity)
}
pub(super) fn validate_generation(
    store: &SqliteStore,
    identity: &SparseIdentity,
) -> Result<(), PortError> {
    let identity_json = identity_json(identity)?;
    let expected_namespace = serde_json::to_string(&identity.namespace).map_err(json_error)?;
    let expected_fingerprint = serde_json::to_string(&identity.fingerprint).map_err(json_error)?;
    let connection = store.lock()?;
    let row: Option<(i64, i64, String, String, String)> = connection
        .query_row(
            "SELECT generation_id, corpus_snapshot, namespace_json, fingerprint_json, lifecycle
             FROM learned_sparse_projections WHERE identity_json = ?1",
            params![identity_json],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(to_port_error)?;
    let Some((generation_id, corpus_snapshot, namespace_json, fingerprint_json, lifecycle)) = row
    else {
        return Err(PortError::Conflict {
            message: "sparse projection generation row is missing".to_string(),
        });
    };
    if generation_id != to_i64(identity.generation_id.value())?
        || corpus_snapshot != to_i64(identity.corpus_snapshot.value())?
        || namespace_json != expected_namespace
        || fingerprint_json != expected_fingerprint
    {
        return Err(PortError::Conflict {
            message: "sparse projection metadata does not match its identity".to_string(),
        });
    }
    lifecycle_from_json(&lifecycle)?;
    Ok(())
}

pub(super) fn replace_documents(
    store: &SqliteStore,
    identity: &SparseIdentity,
    documents: &[SparseDocument],
    clear_first: bool,
) -> Result<(), PortError> {
    validate_documents(identity, documents)?;
    let identity_json = identity_json(identity)?;
    let mut connection = store.lock()?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    ensure_mutable_transaction(&transaction, &identity_json)?;
    if clear_first {
        transaction
            .execute(
                "DELETE FROM learned_sparse_projection_documents WHERE identity_json = ?1",
                params![identity_json],
            )
            .map_err(to_port_error)?;
    }
    for document in documents {
        upsert_document(&transaction, &identity_json, document)?;
    }
    transaction.commit().map_err(to_port_error)
}

pub(super) fn tombstone_documents(
    store: &SqliteStore,
    identity: &SparseIdentity,
    chunk_ids: &[maestria_domain::ChunkId],
) -> Result<(), PortError> {
    let identity_json = identity_json(identity)?;
    let mut connection = store.lock()?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    ensure_mutable_transaction(&transaction, &identity_json)?;
    for chunk_id in chunk_ids {
        transaction
            .execute(
                "UPDATE learned_sparse_projection_documents
                 SET tombstoned = 1 WHERE identity_json = ?1 AND chunk_id = ?2",
                params![identity_json, to_i64(chunk_id.value())?],
            )
            .map_err(to_port_error)?;
    }
    transaction.commit().map_err(to_port_error)
}

pub(super) fn clear_documents(
    store: &SqliteStore,
    identity: &SparseIdentity,
) -> Result<(), PortError> {
    let identity_json = identity_json(identity)?;
    let mut connection = store.lock()?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    ensure_mutable_transaction(&transaction, &identity_json)?;
    transaction
        .execute(
            "DELETE FROM learned_sparse_projection_documents WHERE identity_json = ?1",
            params![identity_json],
        )
        .map_err(to_port_error)?;
    transaction.commit().map_err(to_port_error)
}

fn ensure_mutable_transaction(
    transaction: &Transaction<'_>,
    identity_json: &str,
) -> Result<(), PortError> {
    let lifecycle: String = transaction
        .query_row(
            "SELECT lifecycle FROM learned_sparse_projections WHERE identity_json = ?1",
            params![identity_json],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?
        .ok_or_else(|| PortError::Conflict {
            message: "sparse projection generation row is missing".to_string(),
        })?;
    match lifecycle_from_json(&lifecycle)? {
        IndexLifecycle::Building
        | IndexLifecycle::Evaluated
        | IndexLifecycle::Shadow
        | IndexLifecycle::Active => Ok(()),
        IndexLifecycle::Retired | IndexLifecycle::Collectable | IndexLifecycle::Tombstoned => {
            Err(PortError::Conflict {
                message: "retired sparse projection cannot be modified".to_string(),
            })
        }
    }
}

pub(super) fn load_documents(
    store: &SqliteStore,
    identity: &SparseIdentity,
    max_candidates: u64,
) -> Result<LoadedDocuments, PortError> {
    let identity_json = identity_json(identity)?;
    let candidate_limit =
        usize::try_from(max_candidates).map_err(|_| PortError::InvalidInputContext {
            context: "sparse candidate budget",
            source: "candidate budget exceeds platform range".to_string(),
        })?;
    let probe_limit =
        max_candidates
            .checked_add(1)
            .ok_or_else(|| PortError::InvalidInputContext {
                context: "sparse candidate budget",
                source: "candidate budget cannot be probed safely".to_string(),
            })?;
    let connection = store.lock()?;
    let mut statement = connection
        .prepare(
            "SELECT chunk_id, content_hash, vector_json FROM learned_sparse_projection_documents
             WHERE identity_json = ?1 AND tombstoned = 0 ORDER BY chunk_id LIMIT ?2",
        )
        .map_err(to_port_error)?;
    let rows = statement
        .query_map(params![identity_json, to_i64(probe_limit)?], |row| {
            let chunk_id = row.get::<_, i64>(0)?;
            let content_hash = row.get::<_, String>(1)?;
            let vector_json = row.get::<_, String>(2)?;
            Ok((chunk_id, content_hash, vector_json))
        })
        .map_err(to_port_error)?;
    let mut documents = Vec::new();
    for row in rows {
        let (chunk_id, content_hash, vector_json) = row.map_err(to_port_error)?;
        let vector = decode_vector(&vector_json)?;
        if vector.identity() != identity {
            return Err(PortError::Conflict {
                message: "stored sparse vector identity does not match projection".to_string(),
            });
        }
        ContentHash::new(content_hash).map_err(domain_error)?;
        let encoded_bytes =
            u64::try_from(vector_json.len()).map_err(|_| PortError::Downstream {
                message: "sparse vector JSON length exceeds platform range".to_string(),
            })?;
        documents.push(StoredDocument {
            chunk_id: maestria_domain::ChunkId::new(i64_to_u64(chunk_id)?),
            encoded_bytes,
            vector,
        });
    }
    let candidate_limit_reached = documents.len() > candidate_limit;
    if candidate_limit_reached {
        documents.truncate(candidate_limit);
    }
    Ok(LoadedDocuments {
        documents,
        candidate_limit_reached,
    })
}

fn validate_documents(
    identity: &SparseIdentity,
    documents: &[SparseDocument],
) -> Result<(), PortError> {
    for document in documents {
        if document.vector.identity() != identity {
            return Err(PortError::InvalidInputContext {
                context: "sparse document identity mismatch",
                source: "document identity differs from projection identity".to_string(),
            });
        }
    }
    Ok(())
}

fn upsert_document(
    transaction: &Transaction<'_>,
    identity_json: &str,
    document: &SparseDocument,
) -> Result<(), PortError> {
    let vector_json = encode_vector(&document.vector)?;
    transaction
        .execute(
            "INSERT INTO learned_sparse_projection_documents
             (identity_json, chunk_id, content_hash, vector_json, tombstoned)
             VALUES (?1, ?2, ?3, ?4, 0)
             ON CONFLICT(identity_json, chunk_id) DO UPDATE SET
                 content_hash = excluded.content_hash,
                 vector_json = excluded.vector_json,
                 tombstoned = 0",
            params![
                identity_json,
                to_i64(document.chunk_id.value())?,
                document.content_hash.as_str(),
                vector_json,
            ],
        )
        .map_err(to_port_error)?;
    Ok(())
}

pub(super) fn encode_vector(vector: &SparseVector) -> Result<String, PortError> {
    let stored = StoredVector {
        identity: vector.identity().clone(),
        terms: vector
            .terms()
            .iter()
            .map(|term| StoredTerm {
                term_id: term.term_id(),
                weight: term.weight(),
            })
            .collect(),
    };
    let encoded = serde_json::to_string(&stored).map_err(json_error)?;
    ensure_vector_size(&encoded)?;
    Ok(encoded)
}
fn ensure_vector_size(input: &str) -> Result<(), PortError> {
    if input.len() > MAX_SPARSE_VECTOR_BYTES {
        return Err(PortError::InvalidInputContext {
            context: "sparse vector persistence",
            source: "serialized vector exceeds the durable byte limit".to_string(),
        });
    }
    Ok(())
}

pub(super) fn decode_vector(input: &str) -> Result<SparseVector, PortError> {
    ensure_vector_size(input)?;
    let stored: StoredVector = serde_json::from_str(input).map_err(json_error)?;
    let terms = stored
        .terms
        .into_iter()
        .map(|term| SparseTermWeight::new(term.term_id, term.weight))
        .collect::<Result<Vec<_>, _>>()?;
    SparseVector::new(stored.identity, terms)
}

pub(super) fn lifecycle_json(lifecycle: IndexLifecycle) -> Result<String, PortError> {
    serde_json::to_string(&lifecycle).map_err(json_error)
}

pub(super) fn lifecycle_from_json(input: &str) -> Result<IndexLifecycle, PortError> {
    serde_json::from_str(input).map_err(json_error)
}

pub(super) fn to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| PortError::InvalidInputContext {
        context: "sparse projection identifier",
        source: "identifier exceeds SQLite integer range".to_string(),
    })
}

pub(super) fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value).map_err(|_| PortError::Downstream {
        message: "SQLite sparse projection identifier is negative".to_string(),
    })
}

fn json_error(error: serde_json::Error) -> PortError {
    PortError::InvalidInputContext {
        context: "sparse projection JSON",
        source: error.to_string(),
    }
}

fn domain_error(error: impl std::fmt::Display) -> PortError {
    PortError::InvalidInputContext {
        context: "sparse projection content hash",
        source: error.to_string(),
    }
}
