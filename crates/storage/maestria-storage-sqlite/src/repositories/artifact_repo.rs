use maestria_domain::{Artifact, ArtifactId, CardId, ChunkId, ClaimId, EvidenceId, IndexStatus};
use maestria_ports::{ArtifactRepository, PortError};
use rusqlite::OptionalExtension;
use rusqlite::params;

use super::{load_id_set, replace_id_set};
use crate::sqlite_store::{json_error, to_port_error, u64_to_i64};

impl ArtifactRepository for crate::SqliteStore {
    fn get(&self, artifact_id: ArtifactId) -> Result<Option<Artifact>, PortError> {
        let connection = self.lock()?;
        let row = connection
            .query_row(
                "SELECT title, content_hash, index_status, parse_status, security_json FROM artifacts WHERE id = ?1",
                params![u64_to_i64(artifact_id.value())?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(to_port_error)?;

        let Some((title, content_hash, index_status, parse_status, security_json)) = row else {
            return Ok(None);
        };

        let content_hash = content_hash
            .map(|value| {
                maestria_domain::ContentHash::new(value).map_err(|error| {
                    PortError::InvalidInputContext {
                        context: "decode stored artifact content hash",
                        source: error.to_string(),
                    }
                })
            })
            .transpose()?;

        let index_status = match index_status.as_str() {
            "unindexed" => IndexStatus::Unindexed,
            "pending" => IndexStatus::Pending,
            "indexed" => IndexStatus::Indexed,
            other => {
                return Err(PortError::InternalContext {
                    context: "unknown stored index_status",
                    source: other.to_string(),
                });
            }
        };

        let parse_status = match parse_status.as_deref() {
            Some("parsed") => Some(maestria_domain::ParseStatus::Parsed),
            Some("unsupported") => Some(maestria_domain::ParseStatus::Unsupported),
            Some("failed") => Some(maestria_domain::ParseStatus::Failed),
            Some("metadata_only") => Some(maestria_domain::ParseStatus::MetadataOnly),
            Some("needs_ocr") => Some(maestria_domain::ParseStatus::NeedsOcr),
            Some("quarantined") => Some(maestria_domain::ParseStatus::Quarantined),
            None | Some("none") | Some("") => None,
            Some(other) => {
                return Err(PortError::InternalContext {
                    context: "unknown stored parse_status",
                    source: other.to_string(),
                });
            }
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
            index_status,
            content_hash,
            parse_status,
            security: serde_json::from_str(&security_json).map_err(json_error)?,
        }))
    }

    fn put(&self, artifact: Artifact) -> Result<(), PortError> {
        self.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO artifacts (id, title, content_hash, index_status, parse_status, security_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(id) DO UPDATE SET
                         title = excluded.title,
                         content_hash = excluded.content_hash,
                         index_status = excluded.index_status,
                         parse_status = excluded.parse_status,
                         security_json = excluded.security_json",
                    params![
                        u64_to_i64(artifact.id.value())?,
                        artifact.title,
                        artifact.content_hash.map(|hash| hash.as_str().to_owned()),
                        index_status_to_text(artifact.index_status),
                        match artifact.parse_status {
                            Some(maestria_domain::ParseStatus::Parsed) => "parsed",
                            Some(maestria_domain::ParseStatus::Unsupported) => "unsupported",
                            Some(maestria_domain::ParseStatus::Failed) => "failed",
                            Some(maestria_domain::ParseStatus::MetadataOnly) => "metadata_only",
                            Some(maestria_domain::ParseStatus::NeedsOcr) => "needs_ocr",
                            Some(maestria_domain::ParseStatus::Quarantined) => "quarantined",
                            None => "none",
                        },
                        serde_json::to_string(&artifact.security).map_err(json_error)?,
                    ],
                )
                .map_err(to_port_error)?;

            replace_id_set(
                transaction,
                "artifact_chunks",
                artifact.id,
                artifact.chunk_ids.iter().map(|id| id.value()),
            )?;
            replace_id_set(
                transaction,
                "artifact_cards",
                artifact.id,
                artifact.card_ids.iter().map(|id| id.value()),
            )?;
            replace_id_set(
                transaction,
                "artifact_claims",
                artifact.id,
                artifact.claim_ids.iter().map(|id| id.value()),
            )?;
            replace_id_set(
                transaction,
                "artifact_evidences",
                artifact.id,
                artifact.evidence_ids.iter().map(|id| id.value()),
            )
        })
    }
}

fn index_status_to_text(status: IndexStatus) -> &'static str {
    match status {
        IndexStatus::Unindexed => "unindexed",
        IndexStatus::Pending => "pending",
        IndexStatus::Indexed => "indexed",
    }
}
