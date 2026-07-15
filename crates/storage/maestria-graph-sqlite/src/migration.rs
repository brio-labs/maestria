use maestria_ports::PortError;
use rusqlite::{Connection, OptionalExtension};

use super::conversion::to_port_error;

pub(super) const SCHEMA_VERSION: i64 = 2;

pub(super) fn migrate(connection: &mut Connection) -> Result<(), PortError> {
    let tx = connection.transaction().map_err(to_port_error)?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS graph_projection_schema (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             version INTEGER NOT NULL
         );",
    )
    .map_err(to_port_error)?;

    let current_version: Option<i64> = tx
        .query_row(
            "SELECT version FROM graph_projection_schema WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(to_port_error)?;

    match current_version {
        Some(v) if v == SCHEMA_VERSION => {}
        Some(1) => migrate_v1_to_v2(&tx)?,
        Some(v) => {
            return Err(PortError::Internal {
                message: format!("unsupported graph projection schema version {v}"),
            });
        }
        None => {
            create_initial_schema(&tx)?;
        }
    }

    validate_relations_columns(&tx, 9)?;
    validate_relations_indexes(&tx)?;

    tx.commit().map_err(to_port_error)?;
    Ok(())
}
fn migrate_v1_to_v2(conn: &Connection) -> Result<(), PortError> {
    validate_relations_columns(conn, 8)?;
    conn.execute(
        "ALTER TABLE relations ADD COLUMN security_json TEXT NOT NULL DEFAULT '{\"trust_zone\":\"Untrusted\",\"authority\":\"External\",\"integrity\":\"Unverified\",\"sensitivity\":\"Internal\",\"review_status\":\"Unreviewed\",\"quarantined\":false,\"prompt_injection_risk\":false,\"poisoning_flags\":[],\"read_allowed\":true,\"write_allowed\":false,\"scope_id\":null}'",
        [],
    )
    .map_err(to_port_error)?;
    conn.execute(
        "UPDATE graph_projection_schema SET version = 2 WHERE id = 1",
        [],
    )
    .map_err(to_port_error)?;
    Ok(())
}

fn create_initial_schema(conn: &Connection) -> Result<(), PortError> {
    conn.execute_batch(
        r#"CREATE TABLE relations (
             id TEXT PRIMARY KEY,
             source_type TEXT NOT NULL,
             source_id TEXT NOT NULL,
             kind TEXT NOT NULL,
             target_type TEXT NOT NULL,
             target_id TEXT NOT NULL,
             evidence_id TEXT,
             confidence_milli INTEGER NOT NULL,
             security_json TEXT NOT NULL DEFAULT '{"trust_zone":"Untrusted","authority":"External","integrity":"Unverified","sensitivity":"Internal","review_status":"Unreviewed","quarantined":false,"prompt_injection_risk":false,"poisoning_flags":[],"read_allowed":true,"write_allowed":false,"scope_id":null}'
         );
         CREATE INDEX idx_relations_source
             ON relations(source_type, source_id);
         CREATE INDEX idx_relations_target
             ON relations(target_type, target_id);
         INSERT INTO graph_projection_schema (id, version) VALUES (1, 2);"#,
    )
    .map_err(to_port_error)
}

fn validate_relations_columns(conn: &Connection, expected: usize) -> Result<(), PortError> {
    let mut col_stmt = conn
        .prepare("PRAGMA table_info(relations)")
        .map_err(to_port_error)?;
    let mut cols_found = 0;
    let mut rows = col_stmt.query([]).map_err(to_port_error)?;
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let name: String = row.get(1).map_err(to_port_error)?;
        let ty: String = row.get(2).map_err(to_port_error)?;
        let ty = ty.to_uppercase();

        let valid = match name.as_str() {
            "id" | "source_type" | "source_id" | "kind" | "target_type" | "target_id"
            | "evidence_id" => ty == "TEXT",
            "confidence_milli" => ty == "INTEGER",
            "security_json" => ty == "TEXT",
            _ => false,
        };
        if valid {
            cols_found += 1;
        }
    }
    if cols_found != expected {
        return Err(PortError::Internal {
            message: "relations table is malformed or missing columns".to_string(),
        });
    }
    Ok(())
}

fn validate_relations_indexes(conn: &Connection) -> Result<(), PortError> {
    let mut idx_stmt = conn
        .prepare("PRAGMA index_list(relations)")
        .map_err(to_port_error)?;
    let mut rows = idx_stmt.query([]).map_err(to_port_error)?;
    let mut has_source_idx = false;
    let mut has_target_idx = false;

    while let Some(row) = rows.next().map_err(to_port_error)? {
        let idx_name: String = row.get(1).map_err(to_port_error)?;
        if idx_name == "idx_relations_source" || idx_name == "idx_relations_target" {
            let mut info_stmt = conn
                .prepare(&format!("PRAGMA index_info({})", idx_name))
                .map_err(to_port_error)?;
            let mut info_rows = info_stmt.query([]).map_err(to_port_error)?;
            let mut cols = Vec::new();
            while let Some(info_row) = info_rows.next().map_err(to_port_error)? {
                let col_name: String = info_row.get(2).map_err(to_port_error)?;
                cols.push(col_name);
            }

            if idx_name == "idx_relations_source" && cols == ["source_type", "source_id"] {
                has_source_idx = true;
            } else if idx_name == "idx_relations_target" && cols == ["target_type", "target_id"] {
                has_target_idx = true;
            }
        }
    }

    if !has_source_idx || !has_target_idx {
        return Err(PortError::Internal {
            message: "relations table is missing required indexes".to_string(),
        });
    }
    Ok(())
}
