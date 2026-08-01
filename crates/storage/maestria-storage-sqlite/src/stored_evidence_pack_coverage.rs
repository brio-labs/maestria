//! Wire mirrors of the coverage records of `maestria_domain`:
//! `SourceIndependenceRecord` and `EvidenceFreshnessRecord`.
//!
//! These types serialize the per-evidence independence and freshness
//! annotations attached to an evidence pack. Identifier fields are
//! flattened to raw `u64` values; the shared `FreshnessStatus` delegates
//! to its stored mirror in `stored_search`.

use crate::payloads::stored_search::StoredFreshnessStatus;
use maestria_domain::{
    DuplicateClusterId, EvidenceFreshnessRecord, EvidenceId, SourceIndependenceRecord,
};
use serde::{Deserialize, Serialize};

/// Wire mirror of `maestria_domain::SourceIndependenceRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSourceIndependenceRecord {
    pub(crate) source_key: String,
    pub(crate) evidence_ids: Vec<u64>,
    pub(crate) duplicate_cluster: Option<u64>,
}

impl StoredSourceIndependenceRecord {
    pub(crate) fn from_domain(record: &SourceIndependenceRecord) -> Self {
        Self {
            source_key: record.source_key.clone(),
            evidence_ids: record.evidence_ids.iter().map(|id| id.value()).collect(),
            duplicate_cluster: record.duplicate_cluster.map(|id| id.value()),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<SourceIndependenceRecord, maestria_ports::PortError> {
        Ok(SourceIndependenceRecord {
            source_key: self.source_key,
            evidence_ids: self.evidence_ids.into_iter().map(EvidenceId::new).collect(),
            duplicate_cluster: self.duplicate_cluster.map(DuplicateClusterId::new),
        })
    }
}

/// Wire mirror of `maestria_domain::EvidenceFreshnessRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidenceFreshnessRecord {
    pub(crate) evidence_id: u64,
    pub(crate) status: StoredFreshnessStatus,
}

impl StoredEvidenceFreshnessRecord {
    pub(crate) fn from_domain(record: &EvidenceFreshnessRecord) -> Self {
        Self {
            evidence_id: record.evidence_id.value(),
            status: StoredFreshnessStatus::from_domain(&record.status),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<EvidenceFreshnessRecord, maestria_ports::PortError> {
        Ok(EvidenceFreshnessRecord {
            evidence_id: EvidenceId::new(self.evidence_id),
            status: self.status.try_into_domain()?,
        })
    }
}
