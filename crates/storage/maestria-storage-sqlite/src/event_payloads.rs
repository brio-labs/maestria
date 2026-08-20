mod event_payload_enum;
pub(crate) use event_payload_enum::StoredEventPayload;

use super::stored_content::StoredContentHash;
use super::task_event_payloads::StoredTaskStatus;
use maestria_domain::DomainEvent;
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

impl StoredEventPayload {
    pub(crate) fn from_domain(event: &DomainEvent) -> Result<Self, PortError> {
        Self::try_from_domain_stale(event)
            .or_else(|| Self::try_from_domain_notebook(event))
            .or_else(|| Self::try_from_domain_federation(event))
            .or_else(|| Self::try_from_domain_ocr(event))
            .or_else(|| Self::try_from_domain_artifact(event))
            .or_else(|| Self::try_from_domain_task(event))
            .or_else(|| Self::try_from_domain_claim(event))
            .or_else(|| Self::try_from_domain_memory(event))
            .or_else(|| Self::try_from_domain_misc(event))
            .ok_or_else(|| {
                PortError::internal("encode domain event", "unknown DomainEvent variant")
            })
    }

    pub(crate) fn into_domain(self) -> Result<DomainEvent, PortError> {
        if matches!(&self, Self::EvidenceRecorded { .. }) {
            return self.try_into_domain_evidence();
        }
        self.try_into_domain_stale()
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_notebook))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_federation))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_ocr))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_artifact))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_task))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_claim))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_memory))
            .or_else(|e| e.or_next(StoredEventPayload::try_into_domain_misc))
            .map_err(|error| match error {
                FamilyDecodeError::Foreign(_) => PortError::InternalContext {
                    context: "decode stored event payload",
                    source: "unknown StoredEventPayload variant".to_string(),
                },
                FamilyDecodeError::Invalid(error) => error,
            })
    }

    pub(crate) fn kind(&self) -> Result<&'static str, PortError> {
        self.try_kind_stale()
            .or_else(|| self.try_kind_notebook())
            .or_else(|| self.try_kind_federation())
            .or_else(|| self.try_kind_ocr())
            .or_else(|| self.try_kind_artifact())
            .or_else(|| self.try_kind_task())
            .or_else(|| self.try_kind_claim())
            .or_else(|| self.try_kind_memory())
            .or_else(|| self.try_kind_misc())
            .ok_or_else(|| {
                PortError::internal(
                    "identify stored event payload kind",
                    "unknown StoredEventPayload variant",
                )
            })
    }

    pub(crate) fn filter_artifact_id(&self) -> Option<u64> {
        self.try_filter_artifact_id_stale()
            .or_else(|| self.try_filter_artifact_id_notebook())
            .or_else(|| self.try_filter_artifact_id_federation())
            .or_else(|| self.try_filter_artifact_id_ocr())
            .or_else(|| self.try_filter_artifact_id_artifact())
            .or_else(|| self.try_filter_artifact_id_task())
            .or_else(|| self.try_filter_artifact_id_claim())
            .or_else(|| self.try_filter_artifact_id_memory())
            .or_else(|| self.try_filter_artifact_id_misc())
    }

    fn try_from_domain_stale(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::SourceBecameStale {
                artifact_id,
                source_path,
                content_hash,
            } => Some(Self::SourceBecameStale {
                artifact_id: artifact_id.value(),
                source_path: source_path.clone(),
                content_hash: StoredContentHash::from_domain(content_hash),
            }),
            _ => None,
        }
    }

    fn try_into_domain_stale(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            Self::SourceBecameStale {
                artifact_id,
                source_path,
                content_hash,
            } => Ok(DomainEvent::SourceBecameStale {
                artifact_id: maestria_domain::ArtifactId::new(artifact_id),
                source_path,
                content_hash: content_hash
                    .try_into_domain()
                    .map_err(FamilyDecodeError::Invalid)?,
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    fn try_kind_stale(&self) -> Option<&'static str> {
        match self {
            Self::SourceBecameStale { .. } => Some("source_became_stale"),
            _ => None,
        }
    }

    fn try_filter_artifact_id_stale(&self) -> Option<u64> {
        match self {
            Self::SourceBecameStale { artifact_id, .. } => Some(*artifact_id),
            _ => None,
        }
    }
}

/// Decode failure from a stored-payload family converter.
///
/// `Foreign` means the variant does not belong to that family and the next
/// family should try it; `Invalid` carries a real decode error that must
/// surface instead of being treated as an unknown variant.
#[derive(Debug)]
pub(crate) enum FamilyDecodeError {
    Foreign(Box<StoredEventPayload>),
    Invalid(PortError),
}

impl FamilyDecodeError {
    /// Hand the payload to the next family unless a real decode error occurred.
    fn or_next(
        self,
        family: fn(StoredEventPayload) -> Result<DomainEvent, FamilyDecodeError>,
    ) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            FamilyDecodeError::Foreign(payload) => family(*payload),
            other => Err(other),
        }
    }
}

/// v3 stored encoding of `maestria_domain::ApprovalOutcome`.
///
/// Statuses use `StoredTaskStatus` because the domain `TaskStatus` carries no
/// serde representation; the variant structure mirrors the domain outcome so
/// replay cannot observe a coordinated-flags state.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredApprovalOutcome {
    Acknowledged {
        task_id: Option<u64>,
        approved: bool,
    },
    TaskTransition {
        task_id: u64,
        approved: bool,
        from_status: StoredTaskStatus,
        to_status: StoredTaskStatus,
    },
}
