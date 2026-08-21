use maestria_domain::{ArtifactId, Card, CardId};
use maestria_ports::{CardRepository, PortError};
use rusqlite::{Row, params};

use crate::sqlite_store::{
    i64_to_u64, json_error, row_opt_str, row_str, to_port_error, u64_to_i64,
};

impl CardRepository for crate::SqliteStore {
    fn get(&self, card_id: CardId) -> Result<Option<Card>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, artifact_id, title, body, node_id, source_span_json, security_json FROM cards WHERE id = ?1")
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(card_id.value())?])
            .map_err(to_port_error)?;
        rows.next()
            .map_err(to_port_error)?
            .map(read_card)
            .transpose()
    }

    fn put(&self, card: Card) -> Result<(), PortError> {
        self.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO cards (id, artifact_id, title, body, node_id, source_span_json, security_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                         artifact_id = excluded.artifact_id,
                         title = excluded.title,
                         body = excluded.body,
                         node_id = excluded.node_id,
                         source_span_json = excluded.source_span_json,
                         security_json = excluded.security_json",
                    params![
                        u64_to_i64(card.id.value())?,
                        u64_to_i64(card.artifact_id.value())?,
                        card.title,
                        card.body,
                        u64_to_i64(card.node_id.value())?,
                        serde_json::to_string(
                            &crate::payloads::provenance_payloads::StoredSourceSpan::from(
                                card.source_span
                            )
                        )
                        .map_err(json_error)?,
                        serde_json::to_string(&card.security).map_err(json_error)?,
                    ],
                )
                .map_err(to_port_error)?;
            Ok(())
        })
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Card>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, title, body, node_id, source_span_json, security_json
                 FROM cards
                 WHERE artifact_id = ?1
                 ORDER BY id ASC",
            )
            .map_err(to_port_error)?;
        let mut rows = statement
            .query(params![u64_to_i64(artifact_id.value())?])
            .map_err(to_port_error)?;
        let mut cards = Vec::new();
        while let Some(row) = rows.next().map_err(to_port_error)? {
            cards.push(read_card(row)?);
        }
        Ok(cards)
    }
}

fn read_card(row: &Row<'_>) -> Result<Card, PortError> {
    let id = CardId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?);
    let node_id = match row.get::<_, Option<i64>>(4).map_err(to_port_error)? {
        Some(value) => value,
        None => {
            let card_id = row.get::<_, i64>(0).map_err(to_port_error)?;
            return Err(PortError::InternalContext {
                context: "card repository row missing node_id",
                source: format!("card_id={card_id}"),
            });
        }
    };
    let source_span_json = row_opt_str(row, 5)?;
    let source_span = match source_span_json {
        Some(json) => {
            serde_json::from_str::<crate::payloads::provenance_payloads::StoredSourceSpan>(json)
                .map_err(json_error)?
                .try_into()?
        }
        None => {
            let card_id = row.get::<_, i64>(0).map_err(to_port_error)?;
            return Err(PortError::InternalContext {
                context: "card repository row missing source_span_json",
                source: format!("card_id={card_id}"),
            });
        }
    };

    let security_str = row_str(row, 6)?;
    let security = serde_json::from_str(security_str).map_err(json_error)?;

    Ok(Card {
        id,
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        node_id: maestria_domain::StructureNodeId::new(i64_to_u64(node_id)?),
        source_span,
        title: row.get::<_, String>(2).map_err(to_port_error)?,
        body: row.get::<_, String>(3).map_err(to_port_error)?,
        security,
    })
}
