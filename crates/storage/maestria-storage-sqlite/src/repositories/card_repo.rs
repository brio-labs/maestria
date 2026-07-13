use std::collections::BTreeSet;

use maestria_domain::{ArtifactId, Card, CardId, ClaimId};
use maestria_ports::{CardRepository, PortError};
use rusqlite::{Connection, Row, Transaction, params};

use crate::{i64_to_u64, to_port_error, u64_to_i64};

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
                "INSERT INTO cards (id, artifact_id, title, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                     artifact_id = excluded.artifact_id,
                     title = excluded.title,
                     body = excluded.body",
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
                "SELECT id, artifact_id, title, body
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
            cards.push(read_card(row, &connection)?);
        }
        Ok(cards)
    }
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
