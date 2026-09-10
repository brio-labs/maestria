use maestria_ports::PortError;
use rusqlite::{Connection, Row, params};

use crate::sqlite_store::to_port_error;

const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;

/// Opaque persisted Stage A or Stage B report. Typed validation belongs to the
/// retrieval boundary; SQLite stores the report plus selection identity only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLateInteractionReport {
    pub stage: String,
    pub evaluation_id: String,
    pub corpus_id: String,
    pub evaluation_date: String,
    pub report_hash: String,
    pub report_json: String,
    pub created_at: String,
}

pub(crate) fn save(
    connection: &Connection,
    stage: &str,
    corpus_id: &str,
    evaluation_id: &str,
    evaluation_date: &str,
    report_hash: &str,
    report_json: &str,
) -> Result<(), PortError> {
    if !matches!(stage, "stage-a" | "stage-b" | "promotion")
        || stage.trim().is_empty()
        || evaluation_id.trim().is_empty()
        || evaluation_date.trim().is_empty()
        || report_hash.trim().is_empty()
        || report_json.trim().is_empty()
        || report_json.len() > MAX_REPORT_BYTES
    {
        return Err(PortError::InvalidInputContext {
            context: "late interaction report validation",
            source: "report identity or bounded JSON payload is invalid".to_string(),
        });
    }
    connection
        .execute(
            "INSERT INTO late_interaction_reports
             (stage, evaluation_id, corpus_id, evaluation_date, report_hash, report_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(stage, evaluation_id) DO UPDATE SET
                 corpus_id = excluded.corpus_id,
                 evaluation_date = excluded.evaluation_date,
                 report_hash = excluded.report_hash,
                 report_json = excluded.report_json",
            params![
                stage,
                evaluation_id,
                corpus_id,
                evaluation_date,
                report_hash,
                report_json
            ],
        )
        .map_err(to_port_error)?;
    Ok(())
}

fn decode_row(row: &Row<'_>) -> rusqlite::Result<StoredLateInteractionReport> {
    Ok(StoredLateInteractionReport {
        stage: row.get(0)?,
        evaluation_id: row.get(1)?,
        corpus_id: row.get(2)?,
        evaluation_date: row.get(3)?,
        report_hash: row.get(4)?,
        report_json: row.get(5)?,
        created_at: row.get(6)?,
    })
}

pub(crate) fn load_latest(
    connection: &Connection,
    stage: &str,
) -> Result<Option<StoredLateInteractionReport>, PortError> {
    let mut statement = connection
        .prepare_cached(
            "SELECT stage, evaluation_id, corpus_id, evaluation_date, report_hash, report_json,
                    created_at
             FROM late_interaction_reports
             WHERE stage = ?1
             ORDER BY created_at DESC, evaluation_id DESC
             LIMIT 1",
        )
        .map_err(to_port_error)?;
    let mut rows = statement.query(params![stage]).map_err(to_port_error)?;
    rows.next()
        .map_err(to_port_error)?
        .map(decode_row)
        .transpose()
        .map_err(to_port_error)
}

pub(crate) fn list(
    connection: &Connection,
    stage: &str,
) -> Result<Vec<StoredLateInteractionReport>, PortError> {
    let mut statement = connection
        .prepare_cached(
            "SELECT stage, evaluation_id, corpus_id, evaluation_date, report_hash, report_json,
                    created_at
             FROM late_interaction_reports
             WHERE stage = ?1
             ORDER BY created_at DESC, evaluation_id DESC",
        )
        .map_err(to_port_error)?;
    let mut rows = statement.query(params![stage]).map_err(to_port_error)?;
    let mut reports = Vec::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        reports.push(decode_row(row).map_err(to_port_error)?);
    }
    Ok(reports)
}

pub(crate) fn remove(
    connection: &Connection,
    stage: &str,
    evaluation_id: &str,
) -> Result<(), PortError> {
    connection
        .execute(
            "DELETE FROM late_interaction_reports WHERE stage = ?1 AND evaluation_id = ?2",
            params![stage, evaluation_id],
        )
        .map_err(to_port_error)?;
    Ok(())
}

impl crate::SqliteStore {
    pub fn save_late_interaction_report(
        &self,
        stage: &str,
        corpus_id: &str,
        evaluation_id: &str,
        evaluation_date: &str,
        report_hash: &str,
        report_json: &str,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        save(
            &connection,
            stage,
            corpus_id,
            evaluation_id,
            evaluation_date,
            report_hash,
            report_json,
        )
    }

    pub fn load_latest_late_interaction_report(
        &self,
        stage: &str,
    ) -> Result<Option<StoredLateInteractionReport>, PortError> {
        let connection = self.lock()?;
        load_latest(&connection, stage)
    }

    pub fn list_late_interaction_reports(
        &self,
        stage: &str,
    ) -> Result<Vec<StoredLateInteractionReport>, PortError> {
        let connection = self.lock()?;
        list(&connection, stage)
    }

    pub fn remove_late_interaction_report(
        &self,
        stage: &str,
        evaluation_id: &str,
    ) -> Result<(), PortError> {
        let connection = self.lock()?;
        remove(&connection, stage, evaluation_id)
    }
    pub fn save_late_interaction_promotion_record(
        &self,
        corpus_id: &str,
        evaluation_id: &str,
        evaluation_date: &str,
        report_hash: &str,
        record_json: &str,
    ) -> Result<(), PortError> {
        self.save_late_interaction_report(
            "promotion",
            corpus_id,
            evaluation_id,
            evaluation_date,
            report_hash,
            record_json,
        )
    }

    pub fn load_latest_late_interaction_promotion_record(
        &self,
    ) -> Result<Option<StoredLateInteractionReport>, PortError> {
        self.load_latest_late_interaction_report("promotion")
    }

    pub fn remove_late_interaction_promotion_record(
        &self,
        evaluation_id: &str,
    ) -> Result<(), PortError> {
        self.remove_late_interaction_report("promotion", evaluation_id)
    }
}
