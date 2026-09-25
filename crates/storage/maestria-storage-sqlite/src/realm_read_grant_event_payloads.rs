use std::path::{Component, Path, PathBuf};

use super::event_payloads::{FamilyDecodeError, StoredEventPayload};
use maestria_domain::{
    DomainEvent, EvidenceId, FederatedAccessRecord, FederatedEvidenceBounds, FederatedReadAccess,
    GrantTokenDigest, MAX_REALM_GRANT_ROOT_BYTES, MAX_REALM_GRANT_ROOTS, QueryId, RealmId,
    RealmReadGrant, RealmReadGrantExpiry, SearchTraceId, Sensitivity,
};
use maestria_ports::PortError;
use serde::{Deserialize, Serialize};

crate::stored_enum! {
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    pub(crate) enum StoredFederatedReadAccess <=> FederatedReadAccess {
        SearchOnly,
        SearchAndOpenEvidence,
    }
}

crate::stored_enum! {
    #[serde(rename_all = "snake_case", deny_unknown_fields)]
    pub(crate) enum StoredFederatedSensitivity <=> Sensitivity {
        Public,
        Internal,
        Confidential,
        Restricted,
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
    pub(crate) fn try_from_domain_federation(
        event: &DomainEvent,
    ) -> Result<Option<Self>, PortError> {
        match event {
            DomainEvent::RealmReadGrantIssued { grant } => {
                let allowed_roots = grant
                    .allowed_roots()
                    .map(encode_allowed_root_paths)
                    .transpose()
                    .map_err(|error| PortError::internal("encode realm read grant roots", error))?;
                Ok(Some(Self::RealmReadGrantIssued {
                    token_digest: grant.token_digest().as_str().to_string(),
                    provider_realm: grant.provider_realm().as_str().to_string(),
                    consumer_realm: grant.consumer_realm().as_str().to_string(),
                    access: StoredFederatedReadAccess::from_domain(grant.access()),
                    max_sensitivity: StoredFederatedSensitivity::from_domain(
                        grant.max_sensitivity(),
                    ),
                    max_results: grant.bounds().max_results() as u64,
                    max_evidence_bytes: grant.bounds().max_evidence_bytes() as u64,
                    expires_at_unix_seconds: grant.expires_at().unix_seconds(),
                    allowed_roots,
                }))
            }
            DomainEvent::RealmReadGrantRevoked { token_digest } => {
                Ok(Some(Self::RealmReadGrantRevoked {
                    token_digest: token_digest.as_str().to_string(),
                }))
            }
            DomainEvent::FederatedReadAccessRecorded {
                token_digest,
                provider_realm,
                consumer_realm,
                record,
            } => Ok(Some(Self::FederatedReadAccessRecorded {
                token_digest: token_digest.as_str().to_string(),
                provider_realm: provider_realm.as_str().to_string(),
                consumer_realm: consumer_realm.as_str().to_string(),
                record: StoredFederatedAccessRecord::from_domain(*record),
            })),
            _ => Ok(None),
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
                expires_at_unix_seconds,
                allowed_roots,
            } => {
                let allowed_roots = allowed_roots
                    .map(decode_allowed_root_strings)
                    .transpose()
                    .map_err(invalid)?;
                let grant = RealmReadGrant::new(
                    parse_digest(token_digest)?,
                    parse_realm(provider_realm)?,
                    parse_realm(consumer_realm)?,
                    access
                        .try_into_domain()
                        .map_err(FamilyDecodeError::Invalid)?,
                    max_sensitivity
                        .try_into_domain()
                        .map_err(FamilyDecodeError::Invalid)?,
                    parse_bounds(max_results, max_evidence_bytes)?,
                    RealmReadGrantExpiry::new(expires_at_unix_seconds).map_err(invalid)?,
                );
                let grant = match allowed_roots {
                    Some(roots) => grant.with_allowed_roots(roots),
                    None => grant,
                };
                Ok(DomainEvent::RealmReadGrantIssued { grant })
            }
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

pub(crate) fn encode_allowed_root_paths(roots: &[PathBuf]) -> Result<Vec<String>, String> {
    if roots.is_empty() {
        return Err("allowed roots must be nonempty".to_string());
    }
    if roots.len() > MAX_REALM_GRANT_ROOTS {
        return Err(format!(
            "allowed roots exceed {MAX_REALM_GRANT_ROOTS} entries"
        ));
    }
    if roots
        .iter()
        .map(|root| root.as_os_str().len())
        .sum::<usize>()
        > MAX_REALM_GRANT_ROOT_BYTES
    {
        return Err(format!(
            "allowed root paths exceed {MAX_REALM_GRANT_ROOT_BYTES} bytes"
        ));
    }

    roots
        .iter()
        .map(|root| {
            let value = root
                .to_str()
                .ok_or_else(|| "allowed root path is not valid UTF-8".to_string())?;
            if !is_canonical_root_path(root) {
                return Err(format!(
                    "allowed root path is not absolute and canonical: {value}"
                ));
            }
            Ok(value.to_owned())
        })
        .collect()
}

pub(crate) fn decode_allowed_root_strings(roots: Vec<String>) -> Result<Vec<PathBuf>, String> {
    if roots.is_empty() {
        return Err("allowed roots must be nonempty".to_string());
    }
    if roots.len() > MAX_REALM_GRANT_ROOTS {
        return Err(format!(
            "allowed roots exceed {MAX_REALM_GRANT_ROOTS} entries"
        ));
    }
    if roots.iter().map(String::len).sum::<usize>() > MAX_REALM_GRANT_ROOT_BYTES {
        return Err(format!(
            "allowed root paths exceed {MAX_REALM_GRANT_ROOT_BYTES} bytes"
        ));
    }

    roots
        .into_iter()
        .map(|value| {
            let root = PathBuf::from(&value);
            if !is_canonical_root_path(&root) {
                return Err(format!(
                    "allowed root path is not absolute and canonical: {value}"
                ));
            }
            Ok(root)
        })
        .collect()
}

fn is_canonical_root_path(path: &Path) -> bool {
    let Some(value) = path.to_str() else {
        return false;
    };
    if !path.is_absolute() || value.contains('\0') {
        return false;
    }

    // Event replay is deliberately filesystem-free; this checks only that the
    // persisted spelling is lexically normalized and absolute.
    let mut normalized = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return false;
        }
        normalized.push(component.as_os_str());
    }
    normalized.as_os_str() == path.as_os_str()
}

fn invalid(error: impl std::fmt::Display) -> FamilyDecodeError {
    FamilyDecodeError::Invalid(PortError::InvalidInputContext {
        context: "decode realm read grant event payload",
        source: error.to_string(),
    })
}
