use std::collections::BTreeSet;

use maestria_domain::{ArtifactId, CardId, ChunkId, EvidenceId};
use maestria_ports::PortError;
use rusqlite::{Transaction, params};

use crate::{
    SqliteStore,
    sqlite_store::{i64_to_u64, to_port_error},
};

impl SqliteStore {
    /// Remove projection rows whose IDs are absent from replayed state.
    ///
    /// The operation runs in one SQLite transaction and only deletes IDs not
    /// present in the typed keep sets. Artifact mappings are removed before
    /// stale artifact parents, while entity upserts happen separately in the
    /// daemon after this cleanup so missing valid rows are still repaired.
    pub fn remove_stale_projection_rows(
        &self,
        artifact_ids: &BTreeSet<ArtifactId>,
        chunk_ids: &BTreeSet<ChunkId>,
        card_ids: &BTreeSet<CardId>,
        evidence_ids: &BTreeSet<EvidenceId>,
    ) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_chunks",
            "DELETE FROM artifact_chunks WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_cards",
            "DELETE FROM artifact_cards WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_claims",
            "DELETE FROM artifact_claims WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_mappings(
            &transaction,
            "SELECT DISTINCT artifact_id FROM artifact_evidences",
            "DELETE FROM artifact_evidences WHERE artifact_id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM artifacts",
            "DELETE FROM artifacts WHERE id = ?1",
            artifact_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM chunks",
            "DELETE FROM chunks WHERE id = ?1",
            chunk_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM cards",
            "DELETE FROM cards WHERE id = ?1",
            card_ids.iter().map(|id| id.value()),
        )?;
        remove_stale_projection_ids(
            &transaction,
            "SELECT id FROM evidence",
            "DELETE FROM evidence WHERE id = ?1",
            evidence_ids.iter().map(|id| id.value()),
        )?;
        transaction.commit().map_err(to_port_error)
    }
}

fn remove_stale_projection_ids(
    transaction: &Transaction<'_>,
    select_sql: &'static str,
    delete_sql: &'static str,
    keep_ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    let keep_ids = keep_ids.collect::<BTreeSet<_>>();
    let mut statement = transaction.prepare(select_sql).map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut stale_ids = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let stored_id = row.get::<_, i64>(0).map_err(to_port_error)?;
        if !keep_ids.contains(&i64_to_u64(stored_id)?) {
            stale_ids.push(stored_id);
        }
    }
    drop(rows);
    drop(statement);

    for stale_id in stale_ids {
        transaction
            .execute(delete_sql, params![stale_id])
            .map_err(to_port_error)?;
    }
    Ok(())
}
fn remove_stale_projection_mappings(
    transaction: &Transaction<'_>,
    select_sql: &'static str,
    delete_sql: &'static str,
    keep_artifact_ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    remove_stale_projection_ids(transaction, select_sql, delete_sql, keep_artifact_ids)
}
