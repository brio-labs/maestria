use std::collections::BTreeSet;

use maestria_domain::{
    Artifact, ArtifactId, Card, CardId, Chunk, ChunkId, ClaimId, DomainEventEnvelope, Evidence,
    EvidenceId, LogicalTick,
};
use maestria_ports::{
    ArtifactRepository, CardRepository, ChunkRepository, EventFilter, EventLog, EvidenceRepository,
    PortError,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::{
    events::{StoredEvent, read_stored_event},
    i64_to_u32, i64_to_u64, map_append_error, optional_i64_to_u64, optional_u64_to_i64,
    payloads::StoredEvidenceKind,
    to_port_error, u64_to_i64,
};

impl ArtifactRepository for crate::SqliteStore {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError> {
        let connection = self.lock()?;
        let title = connection
            .query_row(
                "SELECT title FROM artifacts WHERE id = ?1",
                params![u64_to_i64(artifact_id.value())?],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(to_port_error)?;

        let Some(title) = title else {
            return Ok(None);
        };

        Ok(Some(Artifact {
            id: artifact_id,
            title,
            chunk_ids: load_id_set(&connection, "artifact_chunks", artifact_id, ChunkId::new)?,
            card_ids: load_id_set(&connection, "artifact_cards", artifact_id, CardId::new)?,
            claim_ids: load_id_set(&connection, "artifact_claims", artifact_id, ClaimId::new)?,
            evidence_ids: load_id_set(
                &connection,
                "artifact_evidences",
                artifact_id,
                EvidenceId::new,
            )?,
        }))
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        transaction
            .execute(
                "INSERT INTO artifacts (id, title) VALUES (?1, ?2)\n                 ON CONFLICT(id) DO UPDATE SET title = excluded.title",
                params![u64_to_i64(artifact.id.value())?, artifact.title],
            )
            .map_err(to_port_error)?;

        replace_id_set(
            &transaction,
            "artifact_chunks",
            artifact.id,
            artifact.chunk_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_cards",
            artifact.id,
            artifact.card_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_claims",
            artifact.id,
            artifact.claim_ids.iter().map(|id| id.value()),
        )?;
        replace_id_set(
            &transaction,
            "artifact_evidences",
            artifact.id,
            artifact.evidence_ids.iter().map(|id| id.value()),
        )?;

        transaction.commit().map_err(to_port_error)
    }
}

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
                "INSERT INTO chunks (id, artifact_id, chunk_order, text) VALUES (?1, ?2, ?3, ?4)\n                 ON CONFLICT(id) DO UPDATE SET\n                     artifact_id = excluded.artifact_id,\n                     chunk_order = excluded.chunk_order,\n                     text = excluded.text",
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
                "SELECT id, artifact_id, chunk_order, text\n                 FROM chunks\n                 WHERE artifact_id = ?1\n                 ORDER BY chunk_order ASC, id ASC",
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

impl CardRepository for crate::SqliteStore {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, artifact_id, title, body FROM cards WHERE id = ?1")
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(card_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(|row| read_card(row, &connection))
            .transpose()
    }

    fn put(&self, card: Card) -> Result<(), PortError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        transaction
            .execute(
                "INSERT INTO cards (id, artifact_id, title, body) VALUES (?1, ?2, ?3, ?4)\n                 ON CONFLICT(id) DO UPDATE SET\n                     artifact_id = excluded.artifact_id,\n                     title = excluded.title,\n                     body = excluded.body",
                params![
                    u64_to_i64(card.id.value())?,
                    u64_to_i64(card.artifact_id.value())?,
                    card.title,
                    card.body,
                ],
            )
            .map_err(to_port_error)?;
        replace_card_claims(
            &transaction,
            card.id,
            card.claim_ids.iter().map(|id| id.value()),
        )?;
        transaction.commit().map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, title, body\n                 FROM cards\n                 WHERE artifact_id = ?1\n                 ORDER BY id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut cards = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            cards.push(read_card(row, &connection)?);
        }
        Ok(cards)
    }
}

impl EvidenceRepository for crate::SqliteStore {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at\n                 FROM evidence\n                 WHERE id = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(evidence_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(|row| read_evidence(row))
            .transpose()
    }

    fn put(&self, evidence: Evidence) -> Result<(), PortError> {
        let connection = self.lock()?;
        // Check for existing evidence with this id
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at\n                 FROM evidence\n                 WHERE id = ?1",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(evidence.id.value())?])
            .map_err(to_port_error)?;
        if let Some(row) = rows.next().map_err(to_port_error)? {
            let existing = read_evidence(row)?;
            if existing == evidence {
                return Ok(());
            }
            return Err(PortError::Conflict {
                message: format!(
                    "evidence {} already exists with different content; evidence is immutable",
                    evidence.id.value()
                ),
            });
        }
        drop(rows);
        drop(statement);

        let kind_json = serde_json::to_string(&StoredEvidenceKind::from_domain(&evidence.kind))
            .map_err(crate::json_error)?;
        connection
            .execute(
                "INSERT INTO evidence\n                     (id, artifact_id, claim_id, kind_json, excerpt, observed_at)\n                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    u64_to_i64(evidence.id.value())?,
                    u64_to_i64(evidence.artifact_id.value())?,
                    optional_u64_to_i64(evidence.claim_id.map(|id| id.value()))?,
                    kind_json,
                    evidence.excerpt,
                    u64_to_i64(evidence.observed_at.value())?,
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at\n                 FROM evidence\n                 WHERE artifact_id = ?1\n                 ORDER BY id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut evidences = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            evidences.push(read_evidence(row)?);
        }
        Ok(evidences)
    }
}

impl EventLog for crate::SqliteStore {
    fn append(&self, event: DomainEventEnvelope) -> Result<(), PortError> {
        let record = StoredEvent::from_domain(&event)?;
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(to_port_error)?;
        let (count, max_id, max_sequence, mismatched): (i64, Option<i64>, Option<i64>, i64) =
            transaction
                .query_row(
                    "SELECT COUNT(*), MAX(id), MAX(sequence),
                            COALESCE(SUM(CASE WHEN id != sequence OR id < 1 THEN 1 ELSE 0 END), 0)
                     FROM domain_events",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(to_port_error)?;
        let count = u64::try_from(count).map_err(|_| PortError::Internal {
            message: "stored event count is negative".to_string(),
        })?;
        if count > 0 {
            if mismatched != 0 {
                return Err(PortError::Conflict {
                    message: "stored event log has mismatched ids and sequences".to_string(),
                });
            }
            let max_id = max_id.ok_or_else(|| PortError::Internal {
                message: "stored event log has no maximum id".to_string(),
            })?;
            let max_sequence = max_sequence.ok_or_else(|| PortError::Internal {
                message: "stored event log has no maximum sequence".to_string(),
            })?;
            let max_id = i64_to_u64(max_id)?;
            let max_sequence = i64_to_u64(max_sequence)?;
            if max_id != count || max_sequence != count {
                return Err(PortError::Conflict {
                    message: "stored event log is not contiguous".to_string(),
                });
            }
        }
        let expected_sequence = count + 1;
        if record.id != expected_sequence || record.sequence != expected_sequence {
            return Err(PortError::Conflict {
                message: format!(
                    "expected event id/sequence {expected_sequence}, got id {}, sequence {}",
                    record.id, record.sequence
                ),
            });
        }
        transaction
            .execute(
                "INSERT INTO domain_events \
                     (id, sequence, event_kind, artifact_id, payload_json, payload_version)\n                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    u64_to_i64(record.id)?,
                    u64_to_i64(record.sequence)?,
                    record.kind,
                    optional_u64_to_i64(record.artifact_id)?,
                    record.payload_json,
                    record.payload_version,
                ],
            )
            .map_err(map_append_error)?;
        transaction.commit().map_err(to_port_error)
    }

    fn scan(&self, filter: EventFilter) -> Result<Vec<DomainEventEnvelope>, PortError> {
        let connection = self.lock()?;
        let mut events = Vec::new();

        if let Some(artifact_id) = filter.artifact_id {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json, payload_version\n                     FROM domain_events\n                     WHERE artifact_id = ?1\n                     ORDER BY sequence ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement
                .query(params![u64_to_i64(artifact_id.value())?])
                .map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                events.push(read_stored_event(row)?.into_domain()?);
            }
        } else {
            let mut statement = connection
                .prepare(
                    "SELECT id, sequence, event_kind, artifact_id, payload_json, payload_version\n                     FROM domain_events\n                     ORDER BY sequence ASC",
                )
                .map_err(to_port_error)?;
            let mut rows = statement.query([]).map_err(to_port_error)?;
            while let Some(row) = rows.next().map_err(to_port_error)? {
                events.push(read_stored_event(row)?.into_domain()?);
            }
        }

        Ok(events)
    }
}

fn load_id_set<T>(
    connection: &Connection,
    table: &str,
    artifact_id: ArtifactId,
    make: fn(u64) -> T,
) -> Result<BTreeSet<T>, PortError>
where
    T: Ord,
{
    let mut statement = connection
        .prepare(&format!(
            "SELECT related_id FROM {table} WHERE artifact_id = ?1 ORDER BY related_id"
        ))
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![u64_to_i64(artifact_id.value())?])
        .map_err(to_port_error)?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        let value = i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?;
        ids.insert(make(value));
    }
    Ok(ids)
}

fn replace_id_set(
    transaction: &Transaction<'_>,
    table: &str,
    artifact_id: ArtifactId,
    ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    transaction
        .execute(
            &format!("DELETE FROM {table} WHERE artifact_id = ?1"),
            params![u64_to_i64(artifact_id.value())?],
        )
        .map_err(to_port_error)?;

    for id in ids {
        transaction
            .execute(
                &format!("INSERT INTO {table} (artifact_id, related_id) VALUES (?1, ?2)"),
                params![u64_to_i64(artifact_id.value())?, u64_to_i64(id)?],
            )
            .map_err(to_port_error)?;
    }

    Ok(())
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

fn read_card(row: &Row<'_>, connection: &Connection) -> Result<Card, PortError> {
    let id = CardId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?);
    Ok(Card {
        id,
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        title: row.get::<_, String>(2).map_err(to_port_error)?,
        body: row.get::<_, String>(3).map_err(to_port_error)?,
        claim_ids: load_card_claims(connection, id)?,
    })
}

fn load_card_claims(
    connection: &Connection,
    card_id: CardId,
) -> Result<BTreeSet<ClaimId>, PortError> {
    let mut statement = connection
        .prepare("SELECT claim_id FROM card_claims WHERE card_id = ?1 ORDER BY claim_id")
        .map_err(to_port_error)?;
    let mut rows = statement
        .query(params![u64_to_i64(card_id.value())?])
        .map_err(to_port_error)?;
    let mut ids = BTreeSet::new();
    while let Some(row) = rows.next().map_err(to_port_error)? {
        ids.insert(ClaimId::new(i64_to_u64(
            row.get::<_, i64>(0).map_err(to_port_error)?,
        )?));
    }
    Ok(ids)
}

fn replace_card_claims(
    transaction: &Transaction<'_>,
    card_id: CardId,
    ids: impl Iterator<Item = u64>,
) -> Result<(), PortError> {
    transaction
        .execute(
            "DELETE FROM card_claims WHERE card_id = ?1",
            params![u64_to_i64(card_id.value())?],
        )
        .map_err(to_port_error)?;

    for id in ids {
        transaction
            .execute(
                "INSERT INTO card_claims (card_id, claim_id) VALUES (?1, ?2)",
                params![u64_to_i64(card_id.value())?, u64_to_i64(id)?],
            )
            .map_err(to_port_error)?;
    }

    Ok(())
}

fn read_evidence(row: &Row<'_>) -> Result<Evidence, PortError> {
    let kind_json = row.get::<_, String>(3).map_err(to_port_error)?;
    let kind = serde_json::from_str::<StoredEvidenceKind>(&kind_json)
        .map_err(crate::json_error)?
        .into_domain();
    Ok(Evidence {
        id: EvidenceId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?),
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        claim_id: optional_i64_to_u64(row.get::<_, Option<i64>>(2).map_err(to_port_error)?)?
            .map(ClaimId::new),
        kind,
        excerpt: row.get::<_, String>(4).map_err(to_port_error)?,
        observed_at: LogicalTick::new(i64_to_u64(row.get::<_, i64>(5).map_err(to_port_error)?)?),
    })
}
