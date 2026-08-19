use maestria_domain::{ArtifactId, ClaimId, Evidence, EvidenceId, LogicalTick};
use maestria_ports::{EvidenceRepository, PortError};
use rusqlite::{Row, params};

use crate::{
    payloads::StoredEvidenceKind,
    sqlite_store::{
        i64_to_u64, json_error, optional_i64_to_u64, optional_u64_to_i64, to_port_error, u64_to_i64,
    },
};

impl EvidenceRepository for crate::SqliteStore {
    fn get(&self, evidence_id: EvidenceId) -> Result<Option<Evidence>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at, security_json
                 FROM evidence
                 WHERE id = ?1",
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
        let kind_json = serde_json::to_string(&StoredEvidenceKind::from_domain(&evidence.kind))
            .map_err(json_error)?;
        let security_json = serde_json::to_string(&evidence.security).map_err(json_error)?;
        let evidence_id = u64_to_i64(evidence.id.value())?;
        let artifact_id = u64_to_i64(evidence.artifact_id.value())?;
        let claim_id = optional_u64_to_i64(evidence.claim_id.map(|id| id.value()))?;
        let observed_at = u64_to_i64(evidence.observed_at.value())?;

        self.with_transaction(|transaction| {
            transaction
                .execute(
                    "INSERT INTO evidence
                         (id, artifact_id, claim_id, kind_json, excerpt, observed_at, security_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO NOTHING",
                    params![
                        evidence_id,
                        artifact_id,
                        claim_id,
                        &kind_json,
                        &evidence.excerpt,
                        observed_at,
                        &security_json,
                    ],
                )
                .map_err(to_port_error)?;

            let existing = {
                let mut statement = transaction
                    .prepare(
                        "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at, security_json
                         FROM evidence
                         WHERE id = ?1",
                    )
                    .map_err(to_port_error)?;
                let mut rows = statement
                    .query(params![evidence_id])
                    .map_err(to_port_error)?;
                rows.next()
                    .map_err(to_port_error)?
                    .map(read_evidence)
                    .transpose()?
            };

            match existing {
                Some(existing) if existing == evidence => Ok(()),
                Some(_) => Err(PortError::Conflict {
                    message: format!(
                        "evidence {} already exists with different content; evidence is immutable",
                        evidence.id.value()
                    ),
                }),
                None => Err(PortError::internal(
                    "verify inserted evidence",
                    "evidence row was not inserted",
                )),
            }
        })
    }

    fn replace(&self, evidence: Evidence) -> Result<(), PortError> {
        let connection = self.lock()?;
        let kind_json = serde_json::to_string(&StoredEvidenceKind::from_domain(&evidence.kind))
            .map_err(json_error)?;
        connection
            .execute(
                "INSERT OR REPLACE INTO evidence
                     (id, artifact_id, claim_id, kind_json, excerpt, observed_at, security_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    u64_to_i64(evidence.id.value())?,
                    u64_to_i64(evidence.artifact_id.value())?,
                    optional_u64_to_i64(evidence.claim_id.map(|id| id.value()))?,
                    kind_json,
                    evidence.excerpt,
                    u64_to_i64(evidence.observed_at.value())?,
                    serde_json::to_string(&evidence.security).map_err(json_error)?,
                ],
            )
            .map(|_| ())
            .map_err(to_port_error)
    }

    fn list_for_artifact(&self, artifact_id: ArtifactId) -> Result<Vec<Evidence>, PortError> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, artifact_id, claim_id, kind_json, excerpt, observed_at, security_json
                 FROM evidence
                 WHERE artifact_id = ?1
                 ORDER BY id ASC",
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

fn read_evidence(row: &Row<'_>) -> Result<Evidence, PortError> {
    let kind_json = row.get::<_, String>(3).map_err(to_port_error)?;
    let kind = serde_json::from_str::<StoredEvidenceKind>(&kind_json)
        .map_err(json_error)?
        .try_into_domain()
        .map_err(|error| PortError::internal("decode stored evidence kind", error.to_string()))?;
    let security_json = row.get::<_, String>(6).map_err(to_port_error)?;
    let security = serde_json::from_str(&security_json).map_err(json_error)?;

    Ok(Evidence {
        id: EvidenceId::new(i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?),
        artifact_id: ArtifactId::new(i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?),
        claim_id: optional_i64_to_u64(row.get::<_, Option<i64>>(2).map_err(to_port_error)?)?
            .map(ClaimId::new),
        kind,
        excerpt: row.get::<_, String>(4).map_err(to_port_error)?,
        observed_at: LogicalTick::new(i64_to_u64(row.get::<_, i64>(5).map_err(to_port_error)?)?),
        security,
    })
}
