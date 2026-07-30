use std::num::NonZeroUsize;

use maestria_ports::{
    LearnedSparseObservationRepository, LearnedSparseShadowObservation,
    MAX_LEARNED_SPARSE_SHADOW_BYTES, MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS,
};
use rusqlite::{Connection, Transaction, params};

use crate::sqlite_store::{json_error, to_port_error};

const TABLE: &str = "learned_sparse_shadow_observations";

pub(crate) fn append(
    connection: &Connection,
    observation: LearnedSparseShadowObservation,
) -> Result<(), maestria_ports::PortError> {
    let encoded = encode_observation(&observation)?;
    let transaction = connection.unchecked_transaction().map_err(to_port_error)?;
    insert_encoded(&transaction, &observation, &encoded)?;
    transaction.commit().map_err(to_port_error)
}

pub(crate) fn scan(
    connection: &Connection,
    limit: NonZeroUsize,
) -> Result<Vec<LearnedSparseShadowObservation>, maestria_ports::PortError> {
    let limit = i64::try_from(limit.get()).map_err(|error| {
        maestria_ports::PortError::InvalidInputContext {
            context: "learned-sparse shadow scan limit",
            source: error.to_string(),
        }
    })?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT observation_json FROM {TABLE} ORDER BY id DESC LIMIT ?1"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement.query(params![limit]).map_err(to_port_error)?;
    let mut observations = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let encoded = row.get::<_, String>(0).map_err(to_port_error)?;
        observations.push(decode_observation(&encoded)?);
    }
    observations.reverse();
    Ok(observations)
}

pub(crate) fn replace(
    connection: &mut Connection,
    observations: Vec<LearnedSparseShadowObservation>,
) -> Result<(), maestria_ports::PortError> {
    if observations.len() > MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS {
        return Err(maestria_ports::PortError::InvalidInputContext {
            context: "learned-sparse shadow observation retention",
            source: "observation record cap exceeded".to_string(),
        });
    }
    let encoded = observations
        .iter()
        .map(|observation| encode_observation(observation).map(|json| (observation, json)))
        .collect::<Result<Vec<_>, _>>()?;
    let transaction = connection.transaction().map_err(to_port_error)?;
    transaction
        .execute(&format!("DELETE FROM {TABLE}"), [])
        .map_err(to_port_error)?;
    for (observation, json) in encoded {
        insert_encoded(&transaction, observation, &json)?;
    }
    transaction.commit().map_err(to_port_error)
}

pub(crate) fn prune(
    connection: &Connection,
    keep: NonZeroUsize,
) -> Result<(), maestria_ports::PortError> {
    let keep = i64::try_from(keep.get()).map_err(|error| {
        maestria_ports::PortError::InvalidInputContext {
            context: "learned-sparse shadow retention limit",
            source: error.to_string(),
        }
    })?;
    let mut statement = connection
        .prepare(&format!("SELECT id FROM {TABLE} ORDER BY id ASC"))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut ids = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        ids.push(row.get::<_, i64>(0).map_err(to_port_error)?);
    }
    drop(rows);
    drop(statement);
    let retained_start = match usize::try_from(keep) {
        Ok(keep) if keep < ids.len() => ids.len() - keep,
        _ => return Ok(()),
    };
    for id in ids.into_iter().take(retained_start) {
        connection
            .execute(&format!("DELETE FROM {TABLE} WHERE id = ?1"), params![id])
            .map_err(to_port_error)?;
    }
    Ok(())
}

fn insert_encoded(
    transaction: &Transaction<'_>,
    observation: &LearnedSparseShadowObservation,
    encoded: &str,
) -> Result<(), maestria_ports::PortError> {
    transaction
        .execute(
            &format!(
                "INSERT INTO {TABLE} \
                 (schema_version, query_id, query_class, corpus_snapshot, index_generation, elapsed_ms, observation_json) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![
                i64::from(observation.schema_version),
                i64_from_u64(observation.query_id.value(), "query id")?,
                serde_json::to_string(&observation.query_class).map_err(json_error)?,
                i64_from_u64(observation.corpus_snapshot.value(), "corpus snapshot")?,
                i64_from_u64(observation.index_generation.value(), "index generation")?,
                i64_from_u64(observation.elapsed_ms, "elapsed time")?,
                encoded,
            ],
        )
        .map_err(to_port_error)?;
    Ok(())
}

fn encode_observation(
    observation: &LearnedSparseShadowObservation,
) -> Result<String, maestria_ports::PortError> {
    observation
        .validate()
        .map_err(|error| maestria_ports::PortError::InvalidInputContext {
            context: "learned-sparse shadow observation",
            source: error.to_string(),
        })?;
    let encoded = serde_json::to_string(observation).map_err(json_error)?;
    if encoded.len() > MAX_LEARNED_SPARSE_SHADOW_BYTES {
        return Err(maestria_ports::PortError::InvalidInputContext {
            context: "learned-sparse shadow observation",
            source: format!(
                "serialized observation exceeds {MAX_LEARNED_SPARSE_SHADOW_BYTES} bytes"
            ),
        });
    }
    Ok(encoded)
}

fn decode_observation(
    encoded: &str,
) -> Result<LearnedSparseShadowObservation, maestria_ports::PortError> {
    let observation: LearnedSparseShadowObservation =
        serde_json::from_str(encoded).map_err(json_error)?;
    observation
        .validate()
        .map_err(|error| maestria_ports::PortError::InternalContext {
            context: "learned-sparse shadow observation is corrupt",
            source: error.to_string(),
        })?;
    Ok(observation)
}

fn i64_from_u64(value: u64, field: &'static str) -> Result<i64, maestria_ports::PortError> {
    i64::try_from(value).map_err(|error| maestria_ports::PortError::InvalidInputContext {
        context: "learned-sparse shadow numeric field",
        source: format!("{field}: {error}"),
    })
}

impl LearnedSparseObservationRepository for crate::SqliteStore {
    fn append_observation(
        &self,
        observation: LearnedSparseShadowObservation,
    ) -> Result<(), maestria_ports::PortError> {
        let connection = self.lock()?;
        append(&connection, observation)
    }

    fn scan_observations(
        &self,
        limit: NonZeroUsize,
    ) -> Result<Vec<LearnedSparseShadowObservation>, maestria_ports::PortError> {
        let connection = self.lock()?;
        scan(&connection, limit)
    }

    fn replace_observations(
        &self,
        observations: Vec<LearnedSparseShadowObservation>,
    ) -> Result<(), maestria_ports::PortError> {
        let mut connection = self.lock()?;
        replace(&mut connection, observations)
    }

    fn prune_observations(&self, keep: NonZeroUsize) -> Result<(), maestria_ports::PortError> {
        let connection = self.lock()?;
        prune(&connection, keep)
    }
}
