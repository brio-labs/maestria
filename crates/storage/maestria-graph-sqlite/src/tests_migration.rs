use crate::conversion::to_port_error;
use crate::migration::migrate;

use maestria_ports::PortError;
use rusqlite::Connection;

#[test]
fn migration_is_idempotent() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    migrate(&mut conn)?; // Initial migration
    migrate(&mut conn)?; // Second should succeed without error (idempotent)
    Ok(())
}

#[test]
fn rejects_unsupported_schema_version() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;

    // Force an unsupported version manually
    conn.execute_batch(
        "CREATE TABLE graph_projection_schema (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             version INTEGER NOT NULL
         );
         INSERT INTO graph_projection_schema (id, version) VALUES (1, 9999);",
    )
    .map_err(to_port_error)?;

    let result = migrate(&mut conn);
    assert!(matches!(
        result,
        Err(PortError::InternalContext {
            context: "unsupported graph projection schema version",
            ref source,
        }) if source == "9999"
    ));
    Ok(())
}
