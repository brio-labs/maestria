use maestria_ports::PortError;
use rusqlite::{Connection, params};

use crate::sqlite_store::to_port_error;

const TABLE: &str = "learned_sparse_promotion_records";

/// One durable, opaque promotion record row.
///
/// The typed `LearnedSparsePromotionRecord` lives in maestria-retrieval; this
/// adapter persists the serialized JSON plus the identity columns the daemon
/// and CLI need to select and validate records without depending on the
/// retrieval crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPromotionRecord {
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
            context: "learned-sparse promotion record",
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

fn decode_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredPromotionRecord> {
    Ok(StoredPromotionRecord {
        evaluation_id: row.get(0)?,
        corpus_id: row.get(1)?,
        evaluation_date: row.get(2)?,
        report_hash: row.get(3)?,
        record_json: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub(crate) fn load_latest(
    connection: &Connection,
) -> Result<Option<StoredPromotionRecord>, PortError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT evaluation_id, corpus_id, evaluation_date, report_hash, record_json, created_at \
             FROM {TABLE} ORDER BY created_at DESC, evaluation_id DESC LIMIT 1"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let record = rows.next().map_err(to_port_error)?.map(decode_row).transpose().map_err(to_port_error)?;
    Ok(record)
}

pub(crate) fn list(
    connection: &Connection,
) -> Result<Vec<StoredPromotionRecord>, PortError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT evaluation_id, corpus_id, evaluation_date, report_hash, record_json, created_at \
             FROM {TABLE} ORDER BY created_at DESC, evaluation_id DESC"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        records.push(decode_row(row).map_err(to_port_error)?);
    }
    Ok(records)
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

pub(crate) fn remove_all(connection: &Connection) -> Result<usize, PortError> {
    let removed = connection
        .execute(&format!("DELETE FROM {TABLE}"), [])
        .map_err(to_port_error)?;
    Ok(removed)
}

impl crate::SqliteStore {
    pub fn save_promotion_record(
        &self,
        corpus_id: &str,
        evaluation_id: &str,
        evaluation_date: &str,
        report_hash: &str,
        record_json: &str,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        save(&connection, corpus_id, evaluation_id, evaluation_date, report_hash, record_json)
    }

    pub fn load_latest_promotion_record(&self) -> Result<Option<StoredPromotionRecord>, PortError> {
        let connection = self.lock()?;
        load_latest(&connection)
    }

    pub fn list_promotion_records(&self) -> Result<Vec<StoredPromotionRecord>, PortError> {
        let connection = self.lock()?;
        list(&connection)
    }

    pub fn remove_promotion_record(&self, evaluation_id: &str) -> Result<(), PortError> {
        let connection = self.lock()?;
        remove(&connection, evaluation_id)
    }

    pub fn remove_all_promotion_records(&self) -> Result<usize, PortError> {
        let connection = self.lock()?;
        remove_all(&connection)
    }
}
