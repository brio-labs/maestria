use maestria_domain::{DomainEventEnvelope, EventId, SequenceNumber};
use maestria_ports::PortError;
use rusqlite::Row;

use crate::{
    legacy::{decode_stored_payload, leaked_kind, upcast_legacy_approval_id},
    payloads::StoredEventPayload,
    sqlite_store::{i64_to_u64, json_error, optional_i64_to_u64, to_port_error},
};

#[derive(Debug, Clone)]
pub(super) struct StoredEvent {
    pub(crate) id: u64,
    pub(crate) sequence: u64,
    pub(crate) kind: &'static str,
    pub(crate) artifact_id: Option<u64>,
    pub(crate) payload_json: String,
    pub(crate) payload_version: i64,
    pub(crate) legacy_approval_id: Option<u64>,
}

impl StoredEvent {
    pub(super) fn from_domain(envelope: &DomainEventEnvelope) -> Result<Self, PortError> {
        let payload = StoredEventPayload::from_domain(&envelope.event)?;
        Ok(Self {
            id: envelope.id.value(),
            sequence: envelope.sequence.value(),
            kind: payload.kind()?,
            artifact_id: payload.filter_artifact_id(),
            payload_json: serde_json::to_string(&payload).map_err(json_error)?,
            payload_version: 3,
            legacy_approval_id: None,
        })
    }
    pub(super) fn raw_search_trace(
        &self,
    ) -> Result<Option<maestria_domain::SearchTraceId>, PortError> {
        if self.kind != "search_knowledge_completed" {
            return Ok(None);
        }
        let value = upcast_legacy_approval_id(&self.payload_json, self.legacy_approval_id)?;
        let payload: StoredEventPayload = serde_json::from_value(value).map_err(json_error)?;
        let StoredEventPayload::SearchKnowledgeCompleted { outcome, .. } = payload else {
            return Err(PortError::InternalContext {
                context: "stored search completion payload",
                source: "event kind does not match payload variant".to_string(),
            });
        };
        Ok(Some(outcome.trace))
    }

    pub(super) fn into_domain(self) -> Result<DomainEventEnvelope, PortError> {
        let value = upcast_legacy_approval_id(&self.payload_json, self.legacy_approval_id)?;
        let payload = decode_stored_payload(value, self.payload_version)?;
        let payload_kind = payload.kind()?;
        if payload_kind != self.kind {
            return Err(PortError::InternalContext {
                context: "stored event kind mismatch",
                source: format!("column {}, payload {}", self.kind, payload_kind),
            });
        }
        if payload.filter_artifact_id() != self.artifact_id {
            return Err(PortError::InternalContext {
                context: "stored event artifact identity mismatch",
                source: "artifact_id column does not match payload".to_string(),
            });
        }
        let mut event = payload.into_domain()?;
        if let maestria_domain::DomainEvent::SearchKnowledgeCompleted { outcome, .. } = &mut event {
            outcome.canonicalize_score_provenance().map_err(|error| {
                PortError::InternalContext {
                    context: "canonicalize retrieval score provenance during replay",
                    source: error.to_string(),
                }
            })?;
        }
        Ok(DomainEventEnvelope {
            id: EventId::new(self.id),
            sequence: SequenceNumber::new(self.sequence),
            event,
        })
    }
}

pub(super) fn read_stored_event(row: &Row<'_>) -> Result<StoredEvent, PortError> {
    Ok(StoredEvent {
        id: i64_to_u64(row.get::<_, i64>(0).map_err(to_port_error)?)?,
        sequence: i64_to_u64(row.get::<_, i64>(1).map_err(to_port_error)?)?,
        kind: leaked_kind(row.get::<_, String>(2).map_err(to_port_error)?)?,
        artifact_id: optional_i64_to_u64(row.get::<_, Option<i64>>(3).map_err(to_port_error)?)?,
        payload_json: row.get::<_, String>(4).map_err(to_port_error)?,
        payload_version: row.get::<_, i64>(5).map_err(to_port_error)?,
        legacy_approval_id: row
            .get::<_, Option<i64>>(6)
            .map_err(to_port_error)?
            .map(u64::try_from)
            .transpose()
            .map_err(|_| PortError::InternalContext {
                context: "decode mapped legacy approval id",
                source: "approval id is negative".to_string(),
            })?,
    })
}
