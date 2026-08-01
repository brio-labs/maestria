use std::num::NonZeroUsize;

use maestria_ports::PortError;

use crate::{SqliteStore, repositories, sqlite_store::json_error};

impl SqliteStore {
    /// Export all validated learned-sparse shadow observations as typed JSON.
    pub fn export_learned_sparse_observations(&self) -> Result<String, PortError> {
        let limit =
            NonZeroUsize::new(i64::MAX as usize).ok_or_else(|| PortError::InternalContext {
                context: "learned-sparse shadow export limit",
                source: "platform usize has no positive value".to_string(),
            })?;
        let connection = self.lock()?;
        let observations = repositories::learned_sparse_observation_repo::scan(&connection, limit)?;
        serde_json::to_string(&observations).map_err(json_error)
    }

    /// Replace learned-sparse shadow observations from validated typed JSON.
    pub fn import_learned_sparse_observations(&self, input: &str) -> Result<(), PortError> {
        let observations = serde_json::from_str(input).map_err(json_error)?;
        let mut connection = self.lock()?;
        repositories::learned_sparse_observation_repo::replace(&mut connection, observations)
    }
}
