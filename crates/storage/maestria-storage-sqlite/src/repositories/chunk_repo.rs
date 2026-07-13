use maestria_domain::{ArtifactId, Chunk, ChunkId};
use maestria_ports::{ChunkRepository, PortError};
use rusqlite::{Row, params};

use crate::{i64_to_u32, i64_to_u64, to_port_error, u64_to_i64};

impl ChunkRepository for crate::SqliteStore {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, artifact_id, chunk_order, text FROM chunks WHERE id = ?1")
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(chunk_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(|row| read_chunk(row))
            .transpose()
    }

    fn put(&self, chunk: Chunk) -> Result<(), PortError> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO chunks (id, artifact_id, chunk_order, text) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     artifact_id = excluded.artifact_id,
                     chunk_order = excluded.chunk_order,
                     text = excluded.text",
                params![
                    u64_to_i64(chunk.id.value())?,
                    u64_to_i64(chunk.artifact_id.value())?,
                    i64::from(chunk.order),
                    chunk.text,
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, chunk_order, text
                 FROM chunks
                 WHERE artifact_id = ?1
                 ORDER BY chunk_order ASC, id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut chunks = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            chunks.push(read_chunk(row)?);
        }
        Ok(chunks)
    }
}

fn read_chunk(row: &Row<'_>) -> Result<Chunk, PortError> {
    let order = i64_to_u32(row.get::<_, i64>(2).map_err(to_port_error)?)?;
    Ok(Chunk {
        id: ChunkId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?),
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        order,
        text: row.get::<_, String>(3).map_err(to_port_error)?,
    })
}
