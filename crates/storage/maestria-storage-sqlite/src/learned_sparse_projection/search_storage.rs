use maestria_domain::ContentHash;
use maestria_ports::PortError;
use rusqlite::params;

use super::storage;
use crate::{SqliteStore, sqlite_store::to_port_error};

#[derive(Debug, Clone, Copy)]
pub(super) struct DocumentMetadata {
    pub(super) chunk_id: maestria_domain::ChunkId,
    pub(super) encoded_bytes: u64,
}

pub(super) enum DocumentLoadDecision {
    Skip,
    Load,
    Stop,
}

pub(super) enum DocumentVisit {
    Continue,
    Stop,
}

pub(super) trait DocumentVisitor {
    fn before_load(
        &mut self,
        document: DocumentMetadata,
    ) -> Result<DocumentLoadDecision, PortError>;

    fn after_load(&mut self, document: storage::StoredDocument)
    -> Result<DocumentVisit, PortError>;
}

pub(super) fn visit_documents(
    store: &SqliteStore,
    identity: &maestria_ports::SparseIdentity,
    max_candidates: u64,
    visitor: &mut dyn DocumentVisitor,
) -> Result<(), PortError> {
    let identity_json = storage::identity_json(identity)?;
    let probe_limit =
        max_candidates
            .checked_add(1)
            .ok_or_else(|| PortError::InvalidInputContext {
                context: "sparse candidate budget",
                source: "candidate budget cannot be probed safely".to_string(),
            })?;
    let metadata = {
        let connection = store.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT chunk_id, content_hash, length(CAST(vector_json AS BLOB))
                 FROM learned_sparse_projection_documents
                 WHERE identity_json = ?1 AND tombstoned = 0
                 ORDER BY chunk_id LIMIT ?2",
            )
            .map_err(to_port_error)?;
        let rows = statement
            .query_map(
                params![identity_json, storage::to_i64(probe_limit)?],
                |row| {
                    let chunk_id = row.get::<_, i64>(0)?;
                    let content_hash = row.get::<_, String>(1)?;
                    let encoded_bytes = row.get::<_, i64>(2)?;
                    Ok((chunk_id, content_hash, encoded_bytes))
                },
            )
            .map_err(to_port_error)?;
        let mut metadata = Vec::new();
        for row in rows {
            let (chunk_id, content_hash, encoded_bytes) = row.map_err(to_port_error)?;
            ContentHash::new(content_hash).map_err(storage::domain_error)?;
            let encoded_bytes =
                u64::try_from(encoded_bytes).map_err(|_| PortError::Downstream {
                    message: "SQLite sparse vector length is negative".to_string(),
                })?;
            if encoded_bytes > storage::MAX_SPARSE_VECTOR_BYTES as u64 {
                return Err(PortError::InvalidInputContext {
                    context: "sparse vector persistence",
                    source: "serialized vector exceeds the durable byte limit".to_string(),
                });
            }
            metadata.push(DocumentMetadata {
                chunk_id: maestria_domain::ChunkId::new(storage::i64_to_u64(chunk_id)?),
                encoded_bytes,
            });
        }
        metadata
    };
    for document in metadata {
        match visitor.before_load(document)? {
            DocumentLoadDecision::Skip => continue,
            DocumentLoadDecision::Stop => break,
            DocumentLoadDecision::Load => {}
        }
        let vector_json = load_vector(store, &identity_json, document.chunk_id)?;
        if vector_json.len() as u64 != document.encoded_bytes {
            return Err(PortError::Conflict {
                message: "sparse document changed during search".to_string(),
            });
        }
        let vector = storage::decode_vector(&vector_json)?;
        if vector.identity() != identity {
            return Err(PortError::Conflict {
                message: "stored sparse vector identity does not match projection".to_string(),
            });
        }
        if !matches!(
            visitor.after_load(storage::StoredDocument {
                chunk_id: document.chunk_id,
                vector,
            })?,
            DocumentVisit::Continue
        ) {
            break;
        }
    }
    Ok(())
}

fn load_vector(
    store: &SqliteStore,
    identity_json: &str,
    chunk_id: maestria_domain::ChunkId,
) -> Result<String, PortError> {
    let connection = store.lock()?;
    connection
        .query_row(
            "SELECT vector_json FROM learned_sparse_projection_documents
             WHERE identity_json = ?1 AND chunk_id = ?2 AND tombstoned = 0",
            params![identity_json, storage::to_i64(chunk_id.value())?],
            |row| row.get(0),
        )
        .map_err(to_port_error)
}
