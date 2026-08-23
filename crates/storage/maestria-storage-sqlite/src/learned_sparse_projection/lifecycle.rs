use maestria_domain::IndexLifecycle;
use maestria_ports::{PortError, SparseIdentity};
use rusqlite::{OptionalExtension, params};

use super::storage::{identity_json, lifecycle_from_json, lifecycle_json, validate_generation};
use crate::{SqliteStore, sqlite_store::to_port_error};

pub(super) fn read(
    store: &SqliteStore,
    identity: &SparseIdentity,
) -> Result<IndexLifecycle, PortError> {
    validate_generation(store, identity)?;
    let identity_json = identity_json(identity)?;
    let connection = store.lock()?;
    let value: Option<String> = connection
        .query_row(
            "SELECT lifecycle FROM learned_sparse_projections WHERE identity_json = ?1",
            params![identity_json],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    value
        .ok_or_else(|| PortError::Conflict {
            message: "sparse projection lifecycle row is missing".to_string(),
        })
        .and_then(|value| lifecycle_from_json(&value))
}

pub(super) fn transition(
    store: &SqliteStore,
    identity: &SparseIdentity,
    expected: IndexLifecycle,
    next: IndexLifecycle,
) -> Result<(), PortError> {
    validate_generation(store, identity)?;
    if !expected.can_transition_to(next) {
        return Err(PortError::Conflict {
            message: format!(
                "invalid sparse projection lifecycle transition {expected:?} -> {next:?}"
            ),
        });
    }
    let identity_json = identity_json(identity)?;
    let expected_json = lifecycle_json(expected)?;
    let next_json = lifecycle_json(next)?;
    let mut connection = store.lock()?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    let current: Option<String> = transaction
        .query_row(
            "SELECT lifecycle FROM learned_sparse_projections WHERE identity_json = ?1",
            params![identity_json],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    if current.as_deref() != Some(expected_json.as_str()) {
        return Err(PortError::Conflict {
            message: "sparse projection lifecycle changed before transition".to_string(),
        });
    }
    if next == IndexLifecycle::Active {
        let retired = lifecycle_json(IndexLifecycle::Retired)?;
        let active = lifecycle_json(IndexLifecycle::Active)?;
        let mut statement = transaction
            .prepare_cached(
                "SELECT identity_json FROM learned_sparse_projections
         WHERE lifecycle = ?1",
            )
            .map_err(to_port_error)?;
        let rows = statement
            .query_map(params![active], |row| row.get::<_, String>(0))
            .map_err(to_port_error)?;
        let mut retired_identities = Vec::new();
        for row in rows {
            let candidate = row.map_err(to_port_error)?;
            let candidate_identity: SparseIdentity =
                serde_json::from_str(&candidate).map_err(json_error)?;
            candidate_identity.validate()?;
            if candidate_identity.namespace == identity.namespace {
                retired_identities.push(candidate);
            }
        }
        drop(statement);
        for retired_identity in retired_identities {
            transaction
                .execute(
                    "UPDATE learned_sparse_projections SET lifecycle = ?1
                     WHERE identity_json = ?2",
                    params![retired, retired_identity],
                )
                .map_err(to_port_error)?;
        }
    }
    transaction
        .execute(
            "UPDATE learned_sparse_projections SET lifecycle = ?1 WHERE identity_json = ?2",
            params![next_json, identity_json],
        )
        .map_err(to_port_error)?;
    transaction.commit().map_err(to_port_error)
}

pub(super) fn collect(store: &SqliteStore, identity: &SparseIdentity) -> Result<(), PortError> {
    validate_generation(store, identity)?;
    let identity_json = identity_json(identity)?;
    let mut connection = store.lock()?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    let current: Option<String> = transaction
        .query_row(
            "SELECT lifecycle FROM learned_sparse_projections WHERE identity_json = ?1",
            params![identity_json],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;
    let Some(current) = current else {
        return Err(PortError::Conflict {
            message: "cannot collect an unknown sparse projection".to_string(),
        });
    };
    let collectable = lifecycle_json(IndexLifecycle::Collectable)?;
    let tombstoned = lifecycle_json(IndexLifecycle::Tombstoned)?;
    if current != collectable && current != tombstoned {
        return Err(PortError::Conflict {
            message: "only collectable or tombstoned sparse projections can be collected"
                .to_string(),
        });
    }
    transaction
        .execute(
            "DELETE FROM learned_sparse_projection_documents WHERE identity_json = ?1",
            params![identity_json],
        )
        .map_err(to_port_error)?;
    transaction
        .execute(
            "UPDATE learned_sparse_projections SET lifecycle = ?1 WHERE identity_json = ?2",
            params![tombstoned, identity_json],
        )
        .map_err(to_port_error)?;
    transaction.commit().map_err(to_port_error)
}

fn json_error(error: serde_json::Error) -> PortError {
    PortError::InvalidInputContext {
        context: "sparse projection lifecycle JSON",
        source: error.to_string(),
    }
}
