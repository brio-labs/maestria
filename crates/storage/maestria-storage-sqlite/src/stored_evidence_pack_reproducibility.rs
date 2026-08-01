//! Wire mirrors of the reproducibility records of `maestria_domain`:
//! `EvidencePackReplayKeyRecord` and `EvidencePackReproducibilityRecord`.
//!
//! These types serialize the evidence-pack replay key and the
//! reproducibility classification of a pack. The replay key flattens
//! identifier fields to raw `u64` values and delegates the shared
//! `RetrievalModelFingerprint` to its stored mirror in `stored_search`.

use crate::payloads::stored_search::StoredRetrievalModelFingerprint;
use maestria_domain::{
    CorpusSnapshotId, EvidencePackReplayKeyRecord, EvidencePackReproducibilityRecord,
    IndexGenerationId, SearchTraceId,
};
use serde::{Deserialize, Serialize};

/// Wire mirror of `maestria_domain::EvidencePackReplayKeyRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidencePackReplayKeyRecord {
    pub(crate) trace: u64,
    pub(crate) corpus_snapshot: u64,
    pub(crate) index_generation: u64,
    pub(crate) fingerprint: StoredRetrievalModelFingerprint,
    pub(crate) policy_fingerprint: String,
}

impl StoredEvidencePackReplayKeyRecord {
    pub(crate) fn from_domain(record: &EvidencePackReplayKeyRecord) -> Self {
        Self {
            trace: record.trace.value(),
            corpus_snapshot: record.corpus_snapshot.value(),
            index_generation: record.index_generation.value(),
            fingerprint: StoredRetrievalModelFingerprint::from_domain(&record.fingerprint),
            policy_fingerprint: record.policy_fingerprint.clone(),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<EvidencePackReplayKeyRecord, maestria_ports::PortError> {
        Ok(EvidencePackReplayKeyRecord {
            trace: SearchTraceId::new(self.trace),
            corpus_snapshot: CorpusSnapshotId::new(self.corpus_snapshot),
            index_generation: IndexGenerationId::new(self.index_generation),
            fingerprint: self.fingerprint.try_into_domain()?,
            policy_fingerprint: self.policy_fingerprint,
        })
    }
}

/// Wire mirror of `maestria_domain::EvidencePackReproducibilityRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredEvidencePackReproducibilityRecord {
    Frozen(StoredEvidencePackReplayKeyRecord),
    LiveNonReproducible { reason: String },
}

impl StoredEvidencePackReproducibilityRecord {
    pub(crate) fn from_domain(record: &EvidencePackReproducibilityRecord) -> Self {
        match record {
            EvidencePackReproducibilityRecord::Frozen(key) => {
                Self::Frozen(StoredEvidencePackReplayKeyRecord::from_domain(key))
            }
            EvidencePackReproducibilityRecord::LiveNonReproducible { reason } => {
                Self::LiveNonReproducible {
                    reason: reason.clone(),
                }
            }
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<EvidencePackReproducibilityRecord, maestria_ports::PortError> {
        Ok(match self {
            Self::Frozen(key) => EvidencePackReproducibilityRecord::Frozen(key.try_into_domain()?),
            Self::LiveNonReproducible { reason } => {
                EvidencePackReproducibilityRecord::LiveNonReproducible { reason }
            }
        })
    }
}
