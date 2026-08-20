use maestria_ports::PortError;
use rusqlite::{Connection, Row, params};

use crate::sqlite_store::to_port_error;

pub(crate) const TABLE_SPARSE: &str = "learned_sparse_promotion_records";
pub(crate) const TABLE_HYBRID: &str = "hybrid_promotion_records";

/// One durable, opaque promotion record row.
///
/// The typed `LearnedSparsePromotionRecord` / `HybridPromotionRecord` lives in
/// maestria-retrieval; this adapter persists the serialized JSON plus the identity
/// columns the daemon and CLI need to select and validate records without depending
/// on the retrieval crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPromotionRecord {
    pub evaluation_id: String,
    pub corpus_id: String,
    pub evaluation_date: String,
    pub report_hash: String,
    pub record_json: String,
    pub created_at: String,
}

pub type StoredHybridPromotionRecord = StoredPromotionRecord;

pub(crate) fn save(
    connection: &Connection,
    table: &'static str,
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
            context: "promotion record validation",
            source: "promotion record contains empty required fields".to_string(),
        });
    }

    connection
        .execute(
            &format!(
                "INSERT INTO {table}
                 (evaluation_id, corpus_id, evaluation_date, report_hash, record_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(evaluation_id) DO UPDATE SET
                     corpus_id = excluded.corpus_id,
                     evaluation_date = excluded.evaluation_date,
                     report_hash = excluded.report_hash,
                     record_json = excluded.record_json"
            ),
            params![
                evaluation_id,
                corpus_id,
                evaluation_date,
                report_hash,
                record_json
            ],
        )
        .map_err(to_port_error)?;
    Ok(())
}

fn decode_row(row: &Row<'_>) -> rusqlite::Result<StoredPromotionRecord> {
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
    table: &'static str,
) -> Result<Option<StoredPromotionRecord>, PortError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT evaluation_id, corpus_id, evaluation_date, report_hash, record_json, created_at
             FROM {table}
             ORDER BY created_at DESC, evaluation_id DESC
             LIMIT 1"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    rows.next()
        .map_err(to_port_error)?
        .map(decode_row)
        .transpose()
        .map_err(to_port_error)
}

pub(crate) fn list(
    connection: &Connection,
    table: &'static str,
) -> Result<Vec<StoredPromotionRecord>, PortError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT evaluation_id, corpus_id, evaluation_date, report_hash, record_json, created_at
             FROM {table}
             ORDER BY created_at DESC, evaluation_id DESC"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement.query([]).map_err(to_port_error)?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        records.push(decode_row(row).map_err(to_port_error)?);
    }
    Ok(records)
}

pub(crate) fn remove(
    connection: &Connection,
    table: &'static str,
    evaluation_id: &str,
) -> Result<(), PortError> {
    connection
        .execute(
            &format!("DELETE FROM {table} WHERE evaluation_id = ?1"),
            params![evaluation_id],
        )
        .map_err(to_port_error)?;
    Ok(())
}

pub(crate) fn remove_all(connection: &Connection, table: &'static str) -> Result<usize, PortError> {
    connection
        .execute(&format!("DELETE FROM {table}"), [])
        .map_err(to_port_error)
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
        save(
            &connection,
            TABLE_SPARSE,
            corpus_id,
            evaluation_id,
            evaluation_date,
            report_hash,
            record_json,
        )
    }

    pub fn load_latest_promotion_record(&self) -> Result<Option<StoredPromotionRecord>, PortError> {
        let connection = self.lock()?;
        load_latest(&connection, TABLE_SPARSE)
    }

    pub fn list_promotion_records(&self) -> Result<Vec<StoredPromotionRecord>, PortError> {
        let connection = self.lock()?;
        list(&connection, TABLE_SPARSE)
    }

    pub fn remove_promotion_record(&self, evaluation_id: &str) -> Result<(), PortError> {
        let connection = self.lock()?;
        remove(&connection, TABLE_SPARSE, evaluation_id)
    }

    pub fn remove_all_promotion_records(&self) -> Result<usize, PortError> {
        let connection = self.lock()?;
        remove_all(&connection, TABLE_SPARSE)
    }

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
            TABLE_HYBRID,
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
        load_latest(&connection, TABLE_HYBRID)
    }
}
