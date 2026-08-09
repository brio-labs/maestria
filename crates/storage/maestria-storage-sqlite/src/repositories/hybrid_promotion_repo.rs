use maestria_ports::PortError;
use rusqlite::{Connection, params};

use crate::sqlite_store::to_port_error;

const TABLE: &str = "hybrid_promotion_records";

/// One durable hybrid (lexical + dense) promotion record row.
///
/// The typed `HybridPromotionRecord` lives in maestria-retrieval; this
/// adapter persists the serialized JSON plus the identity columns the daemon
/// needs to select and validate records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredHybridPromotionRecord {
    pub evaluation_id: String,
    pub corpus_id: String,
    pub evaluation_date: String,
    pub report_hash: String,
    pub record_json: String,
    pub created_at: String,
}

pub(crate) fn save(
    connection: &Connection,
    corpus_id: &str,
    evaluation_id: &str,
    evaluation_date: &str,
    report_hash: &str,
    record_json: &str,
) -> Result<(), PortError> {
    if evaluation_id.trim().is_empty()
        || corpus_id.trim().is_empty()
        || evaluation_date.trim().is_empty()
        || report_hash.trim().is_empty()
        || record_json.trim().is_empty()
    {
        return Err(PortError::InvalidInputContext {
            context: "hybrid promotion record",
            source: "identity and payload fields must be non-empty".to_string(),
        });
    }
    connection
        .execute(
            &format!(
                "INSERT OR REPLACE INTO {TABLE} \
                 (evaluation_id, corpus_id, evaluation_date, report_hash, record_json, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)"
            ),
            params![
                evaluation_id,
                corpus_id,
                evaluation_date,
                report_hash,
                record_json,
            ],
        )
        .map_err(to_port_error)?;
    Ok(())
}

pub(crate) fn load_latest(
    connection: &Connection,
) -> Result<Option<StoredHybridPromotionRecord>, PortError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT evaluation_id, corpus_id, evaluation_date, report_hash, record_json, created_at
             FROM {TABLE} ORDER BY created_at DESC LIMIT 1"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let row = rows.next().map_err(to_port_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(StoredHybridPromotionRecord {
        evaluation_id: row.get(0).map_err(to_port_error)?,
        corpus_id: row.get(1).map_err(to_port_error)?,
        evaluation_date: row.get(2).map_err(to_port_error)?,
        report_hash: row.get(3).map_err(to_port_error)?,
        record_json: row.get(4).map_err(to_port_error)?,
        created_at: row.get(5).map_err(to_port_error)?,
    }))
}

pub(crate) fn remove(connection: &Connection, evaluation_id: &str) -> Result<(), PortError> {
    connection
        .execute(
            &format!("DELETE FROM {TABLE} WHERE evaluation_id = ?1"),
            params![evaluation_id],
        )
        .map_err(to_port_error)?;
    Ok(())
}

impl crate::SqliteStore {
    pub fn save_hybrid_promotion_record(
        &self,
        corpus_id: &str,
        evaluation_id: &str,
        evaluation_date: &str,
        report_hash: &str,
        record_json: &str,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        save(
            &connection,
            corpus_id,
            evaluation_id,
            evaluation_date,
            report_hash,
            record_json,
        )
    }

    pub fn load_latest_hybrid_promotion_record(
        &self,
    ) -> Result<Option<StoredHybridPromotionRecord>, PortError> {
        let connection = self.lock()?;
        load_latest(&connection)
    }

    pub fn remove_hybrid_promotion_record(&self, evaluation_id: &str) -> Result<(), PortError> {
        let connection = self.lock()?;
        remove(&connection, evaluation_id)
    }
}
