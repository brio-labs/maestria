use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use maestria_domain::{
    DomainEvent, EvidenceId, FederatedAccessRecord, FederatedEvidenceBounds, FederatedReadAccess,
    GrantTokenDigest, QueryId, RealmId, RealmReadGrant, SearchTraceId, Sensitivity,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredFederatedReadAccess {
    SearchOnly,
    SearchAndOpenEvidence,
}

impl StoredFederatedReadAccess {
    fn from_domain(value: FederatedReadAccess) -> Self {
        match value {
            FederatedReadAccess::SearchOnly => Self::SearchOnly,
            FederatedReadAccess::SearchAndOpenEvidence => Self::SearchAndOpenEvidence,
        }
    }

    fn into_domain(self) -> FederatedReadAccess {
        match self {
            Self::SearchOnly => FederatedReadAccess::SearchOnly,
            Self::SearchAndOpenEvidence => FederatedReadAccess::SearchAndOpenEvidence,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredFederatedSensitivity {
    Public,
    Internal,
    Confidential,
    Restricted,
}

impl StoredFederatedSensitivity {
    fn from_domain(value: &Sensitivity) -> Self {
        match value {
            Sensitivity::Public => Self::Public,
            Sensitivity::Internal => Self::Internal,
            Sensitivity::Confidential => Self::Confidential,
            Sensitivity::Restricted => Self::Restricted,
        }
    }

    fn into_domain(self) -> Sensitivity {
        match self {
            Self::Public => Sensitivity::Public,
            Self::Internal => Sensitivity::Internal,
            Self::Confidential => Sensitivity::Confidential,
            Self::Restricted => Sensitivity::Restricted,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredFederatedAccessRecord {
    Search { query_id: u64, trace_id: u64 },
    Evidence { evidence_id: u64 },
}

impl StoredFederatedAccessRecord {
    fn from_domain(value: FederatedAccessRecord) -> Self {
        match value {
            FederatedAccessRecord::Search { query_id, trace_id } => Self::Search {
                query_id: query_id.value(),
                trace_id: trace_id.value(),
            },
            FederatedAccessRecord::Evidence { evidence_id } => Self::Evidence {
                evidence_id: evidence_id.value(),
            },
        }
    }

    fn into_domain(self) -> FederatedAccessRecord {
        match self {
            Self::Search { query_id, trace_id } => FederatedAccessRecord::Search {
                query_id: QueryId::new(query_id),
                trace_id: SearchTraceId::new(trace_id),
            },
            Self::Evidence { evidence_id } => FederatedAccessRecord::Evidence {
                evidence_id: EvidenceId::new(evidence_id),
            },
        }
    }
}

impl StoredEventPayload {
    pub(crate) fn try_from_domain_federation(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::RealmReadGrantIssued { grant } => Some(Self::RealmReadGrantIssued {
                token_digest: grant.token_digest().as_str().to_string(),
                provider_realm: grant.provider_realm().as_str().to_string(),
                consumer_realm: grant.consumer_realm().as_str().to_string(),
                access: StoredFederatedReadAccess::from_domain(grant.access()),
                max_sensitivity: StoredFederatedSensitivity::from_domain(grant.max_sensitivity()),
                max_results: grant.bounds().max_results() as u64,
                max_evidence_bytes: grant.bounds().max_evidence_bytes() as u64,
            }),
            DomainEvent::RealmReadGrantRevoked { token_digest } => {
                Some(Self::RealmReadGrantRevoked {
                    token_digest: token_digest.as_str().to_string(),
                })
            }
            DomainEvent::FederatedReadAccessRecorded {
                token_digest,
                provider_realm,
                consumer_realm,
                record,
            } => Some(Self::FederatedReadAccessRecorded {
                token_digest: token_digest.as_str().to_string(),
                provider_realm: provider_realm.as_str().to_string(),
                consumer_realm: consumer_realm.as_str().to_string(),
                record: StoredFederatedAccessRecord::from_domain(*record),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_federation(self) -> Result<DomainEvent, FamilyDecodeError> {
        match self {
            Self::RealmReadGrantIssued {
                token_digest,
                provider_realm,
                consumer_realm,
                access,
                max_sensitivity,
                max_results,
                max_evidence_bytes,
            } => Ok(DomainEvent::RealmReadGrantIssued {
                grant: RealmReadGrant::new(
                    parse_digest(token_digest)?,
                    parse_realm(provider_realm)?,
                    parse_realm(consumer_realm)?,
                    access.into_domain(),
                    max_sensitivity.into_domain(),
                    parse_bounds(max_results, max_evidence_bytes)?,
                ),
            }),
            Self::RealmReadGrantRevoked { token_digest } => {
                Ok(DomainEvent::RealmReadGrantRevoked {
                    token_digest: parse_digest(token_digest)?,
                })
            }
            Self::FederatedReadAccessRecorded {
                token_digest,
                provider_realm,
                consumer_realm,
                record,
            } => Ok(DomainEvent::FederatedReadAccessRecorded {
                token_digest: parse_digest(token_digest)?,
                provider_realm: parse_realm(provider_realm)?,
                consumer_realm: parse_realm(consumer_realm)?,
                record: record.into_domain(),
            }),
            other => Err(FamilyDecodeError::Foreign(Box::new(other))),
        }
    }

    pub(crate) fn try_kind_federation(&self) -> Option<&'static str> {
        match self {
            Self::RealmReadGrantIssued { .. } => Some("realm_read_grant_issued"),
            Self::RealmReadGrantRevoked { .. } => Some("realm_read_grant_revoked"),
            Self::FederatedReadAccessRecorded { .. } => Some("federated_read_access_recorded"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_federation(&self) -> Option<u64> {
        None
    }
}

fn parse_digest(value: String) -> Result<GrantTokenDigest, FamilyDecodeError> {
    GrantTokenDigest::try_from(value).map_err(invalid)
}

fn parse_realm(value: String) -> Result<RealmId, FamilyDecodeError> {
    RealmId::try_from(value).map_err(invalid)
}

fn parse_bounds(
    max_results: u64,
    max_evidence_bytes: u64,
) -> Result<FederatedEvidenceBounds, FamilyDecodeError> {
    let max_results = usize::try_from(max_results).map_err(invalid)?;
    let max_evidence_bytes = usize::try_from(max_evidence_bytes).map_err(invalid)?;
    FederatedEvidenceBounds::try_new(max_results, max_evidence_bytes).map_err(invalid)
}

fn invalid(error: impl std::fmt::Display) -> FamilyDecodeError {
    FamilyDecodeError::Invalid(PortError::InvalidInputContext {
        context: "decode realm read grant event payload",
        source: error.to_string(),
    })
}
