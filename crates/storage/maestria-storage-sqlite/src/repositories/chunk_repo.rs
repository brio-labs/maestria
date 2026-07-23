use maestria_domain::{ArtifactId, Chunk, ChunkId};
use maestria_ports::{ChunkRepository, PortError};
use rusqlite::{Row, params};

use crate::{i64_to_u32, i64_to_u64, to_port_error, u64_to_i64};

impl ChunkRepository for crate::SqliteStore {
    fn get(&self, chunk_id: ChunkId) -> Result<Option<Chunk>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, artifact_id, chunk_order, text, node_id, source_span_json, representations_json FROM chunks WHERE id = ?1")
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
                "INSERT INTO chunks (id, artifact_id, chunk_order, text, node_id, source_span_json, representations_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO UPDATE SET
                     artifact_id = excluded.artifact_id,
                     chunk_order = excluded.chunk_order,
                     text = excluded.text,
                     node_id = excluded.node_id,
                     source_span_json = excluded.source_span_json,
                     representations_json = excluded.representations_json",
                params![
                    u64_to_i64(chunk.id.value())?,
                    u64_to_i64(chunk.artifact_id.value())?,
                    i64::from(chunk.order),
                    chunk.text,
                    u64_to_i64(chunk.node_id.value())?,
                    serde_json::to_string(&crate::payloads::provenance_payloads::StoredSourceSpan::from(chunk.source_span)).map_err(crate::json_error)?,
                    serde_json::to_string(&chunk.representations.into_iter().map(crate::payloads::provenance_payloads::StoredParsedRepresentation::from).collect::<Vec<_>>()).map_err(crate::json_error)?,
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Chunk>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, chunk_order, text, node_id, source_span_json, representations_json
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
    let node_id = match row.get::<_, Option<i64>>(4).map_err(to_port_error)? {
        Some(value) => value,
        None => {
            let _ = ();
            0
        }
    };
    let source_span_json = row.get::<_, Option<String>>(5).map_err(to_port_error)?;
    let source_span = if let Some(json) = source_span_json {
        serde_json::from_str::<crate::payloads::provenance_payloads::StoredSourceSpan>(&json)
            .map_err(crate::json_error)?
            .into()
    } else {
        crate::payloads::provenance_payloads::StoredSourceSpan::default().into()
    };
    let representations_json = row.get::<_, Option<String>>(6).map_err(to_port_error)?;
    let representations = if let Some(json) = representations_json {
        serde_json::from_str::<Vec<crate::payloads::provenance_payloads::StoredParsedRepresentation>>(&json)
            .map_err(crate::json_error)?
            .into_iter()
            .map(Into::into)
            .collect()
    } else {
        Vec::new()
    };

    Ok(Chunk {
        id: ChunkId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?),
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        node_id: maestria_domain::StructureNodeId::new(i64_to_u64(node_id)?),
        source_span,
        representations,
        order,
        text: row.get::<_, String>(3).map_err(to_port_error)?,
    })
}
