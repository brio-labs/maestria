mod approval_repo;
mod artifact_repo;
mod card_repo;
mod chunk_repo;
pub(crate) mod effect_journal_repo;
mod event_log_repo;
mod evidence_repo;
pub(crate) mod learned_sparse_observation_repo;
pub(crate) mod learned_sparse_promotion_repo;
mod realm_read_grant_repo;

use maestria_domain::ArtifactId;
use maestria_ports::PortError;
use maestria_sqlite_support::BindId;
use rusqlite::{Connection, Transaction, params};
use std::collections::BTreeSet;

use crate::sqlite_store::{i64_to_u64, to_port_error, u64_to_i64};

pub(super) fn load_id_set<T: Ord>(
    connection: &Connection,
    table: &'static str,
    artifact_id: ArtifactId,
    make: fn(u64) -> T,
) -> Result<BTreeSet<T>, PortError> {
    let mut statement = connection
        .prepare_cached(&format!(
            "SELECT related_id FROM {table} WHERE artifact_id = ?1 ORDER BY related_id"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![artifact_id.to_sql_param()?])
        .map_err(to_port_error)?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let value = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
        ids.insert(make(value));
    }
    Ok(ids)
}

pub(super) fn replace_id_set(
    transaction: &Transaction<'_>,
    table: &'static str,
    artifact_id: ArtifactId,
    ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    transaction
        .execute(
            &format!("DELETE FROM {table} WHERE artifact_id = ?1"),
            params![artifact_id.to_sql_param()?],
        )
        .map_err(to_port_error)?;

    let mut statement = transaction
        .prepare_cached(&format!(
            "INSERT INTO {table} (artifact_id, related_id) VALUES (?1, ?2)"
        ))
        .map_err(to_port_error)?;

    for id in ids {
        statement
            .execute(params![artifact_id.to_sql_param()?, u64_to_i64(id)?])
            .map_err(to_port_error)?;
    }

    Ok(())
}
