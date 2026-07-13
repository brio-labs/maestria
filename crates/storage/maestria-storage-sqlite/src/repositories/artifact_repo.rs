use std::collections::BTreeSet;

use maestria_domain::{Artifact, ArtifactId, CardId, ChunkId, ClaimId, EvidenceId, IndexStatus};
use maestria_ports::{ArtifactRepository, PortError};
use rusqlite::OptionalExtension;
use rusqlite::{Connection, Transaction, params};

use crate::{i64_to_u64, to_port_error, u64_to_i64};

impl ArtifactRepository for crate::SqliteStore {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError> {
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT title, content_hash, index_status FROM artifacts WHERE id = ?1",
                params![u64_to_i64(artifact_id.value())?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(to_port_error)?;

        let Some((title, content_hash, index_status)) = row else {
            return Ok(None);
        };

        let index_status = match index_status.as_str() {
            "unindexed" => IndexStatus::Unindexed,
            "pending" => IndexStatus::Pending,
            "indexed" => IndexStatus::Indexed,
            other => {
                return Err(PortError::Internal {
                    message: format!("unknown stored index_status {other}"),
                });
            }
        };

        Ok(Some(Artifact {
            id: artifact_id,
            title,
            chunk_ids: load_id_set(&connection, "artifact_chunks", artifact_id, ChunkId::new)?,
            card_ids: load_id_set(&connection, "artifact_cards", artifact_id, CardId::new)?,
            claim_ids: load_id_set(&connection, "artifact_claims", artifact_id, ClaimId::new)?,
            evidence_ids: load_id_set(
                &connection,
                "artifact_evidences",
                artifact_id,
                EvidenceId::new,
            )?,
            index_status,
            content_hash,
        }))
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        transaction
            .execute(
                "INSERT INTO artifacts (id, title, content_hash, index_status)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     title = excluded.title,
                     content_hash = excluded.content_hash,
                     index_status = excluded.index_status",
                params![
                    u64_to_i64(artifact.id.value())?,
                    artifact.title,
                    artifact.content_hash,
                    index_status_to_text(artifact.index_status),
                ],
            )
            .map_err(to_port_error)?;

        replace_id_set(
            &transaction,
            "artifact_chunks",
            artifact.id,
            artifact.chunk_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_cards",
            artifact.id,
            artifact.card_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_claims",
            artifact.id,
            artifact.claim_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_evidences",
            artifact.id,
            artifact.evidence_ids.iter().map(|id| id.value()),
        )?;

        transaction.commit().map_err(to_port_error)
    }
}

fn load_id_set<T>(
    connection: &Connection,
    table: &str,
    artifact_id: ArtifactId,
    make: fn(u64) -> T,
) -> Result<BTreeSet<T>, PortError>
where
    T: Ord,
{
    let mut statement = connection
        .prepare(&format!(
            "SELECT related_id FROM {table} WHERE artifact_id = ?1 ORDER BY related_id"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![u64_to_i64(artifact_id.value())?])
        .map_err(to_port_error)?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let value = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
        ids.insert(make(value));
    }
    Ok(ids)
}

fn replace_id_set(
    transaction: &Transaction<'_>,
    table: &str,
    artifact_id: ArtifactId,
    ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    transaction
        .execute(
            &format!("DELETE FROM {table} WHERE artifact_id = ?1"),
            params![u64_to_i64(artifact_id.value())?],
        )
        .map_err(to_port_error)?;

    for id in ids {
        transaction
            .execute(
                &format!("INSERT INTO {table} (artifact_id, related_id) VALUES (?1, ?2)"),
                params![u64_to_i64(artifact_id.value())?, u64_to_i64(id)?],
            )
            .map_err(to_port_error)?;
    }

    Ok(())
}

fn index_status_to_text(status: IndexStatus) -> &'static str {
    match status {
        IndexStatus::Unindexed => "unindexed",
        IndexStatus::Pending => "pending",
        IndexStatus::Indexed => "indexed",
    }
}
