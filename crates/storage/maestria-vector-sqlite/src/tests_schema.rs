use maestria_ports::PortError;
use rusqlite::Connection;

use crate::encoding::to_port_error;
use crate::schema::{SCHEMA_VERSION, migrate, sqlite_vec_available};

#[test]
fn rejects_unsupported_schema_version() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    conn.execute_batch(
        "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
         INSERT INTO vector_projection_schema (id, version) VALUES (1, 999);",
    )
    .map_err(to_port_error)?;

    match migrate(&mut conn) {
        Err(PortError::InternalContext { context, source }) => {
            assert_eq!(context, "unsupported vector projection schema version");
            assert_eq!(source, "999");
        }
        Err(_) => {
            return Err(PortError::internal(
                "maestria vector sqlite test",
                "Expected unsupported version error, got different error".to_string(),
            ));
        }
        Ok(_) => {
            return Err(PortError::internal(
                "maestria vector sqlite test",
                "Expected error but got Ok".to_string(),
            ));
        }
    }
    Ok(())
}
#[test]
fn rejects_zero_schema_version() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    conn.execute_batch(
        "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
         INSERT INTO vector_projection_schema (id, version) VALUES (1, 0);",
    )
    .map_err(to_port_error)?;

    match migrate(&mut conn) {
        Err(PortError::InternalContext { context, source }) => {
            assert_eq!(context, "unsupported vector projection schema version");
            assert_eq!(source, "0");
        }
        Err(_) => {
            return Err(PortError::internal(
                "maestria vector sqlite test",
                "Expected unsupported version error, got different error".to_string(),
            ));
        }
        Ok(_) => {
            return Err(PortError::internal(
                "maestria vector sqlite test",
                "Expected error but got Ok".to_string(),
            ));
        }
    }
    Ok(())
}

#[test]
fn migrates_version_1_schema_to_current() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    conn.execute_batch(
        "CREATE TABLE vector_projection_schema (id INTEGER PRIMARY KEY, version INTEGER);
         INSERT INTO vector_projection_schema (id, version) VALUES (1, 1);
         CREATE TABLE vector_embeddings (
             chunk_id INTEGER PRIMARY KEY NOT NULL,
             dimension INTEGER NOT NULL,
             embedding BLOB NOT NULL
         );",
    )
    .map_err(to_port_error)?;

    migrate(&mut conn)?;

    let v: i64 = conn
        .query_row(
            "SELECT version FROM vector_projection_schema WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .map_err(to_port_error)?;
    assert_eq!(v, SCHEMA_VERSION);

    // Verify new columns exist by doing a dummy insert
    conn.execute("INSERT INTO vector_embeddings (chunk_id, dimension, embedding, content_hash, provider_id, model, model_version, generation_id, representation, fingerprint) VALUES (1, 1, X'00', 'a', 'provider', 'model', 'b', 'gen', 'rep', 'finger')", []).map_err(to_port_error)?;
    Ok(())
}

#[test]
fn sqlite_vec_detection_verifies_virtual_table() -> Result<(), PortError> {
    let conn = Connection::open_in_memory().map_err(to_port_error)?;
    // Create a regular table named vec_docs with spoofed comment
    conn.execute("CREATE TABLE vec_docs (id INTEGER /* USING VEC0 */)", [])
        .map_err(to_port_error)?;

    assert!(!sqlite_vec_available(&conn)?);

    Ok(())
}
