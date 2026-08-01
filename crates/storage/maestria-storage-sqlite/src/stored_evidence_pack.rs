//! Wire mirrors of `maestria_domain` evidence-pack records.
//!
//! This module is the façade for the evidence-pack wire mirrors: the
//! coverage records (`StoredSourceIndependenceRecord`,
//! `StoredEvidenceFreshnessRecord`) live in
//! `stored_evidence_pack_coverage`, and the reproducibility records
//! (`StoredEvidencePackReplayKeyRecord`,
//! `StoredEvidencePackReproducibilityRecord`) live in
//! `stored_evidence_pack_reproducibility`. Both are re-exported here so
//! consumers keep importing from `crate::payloads::stored_evidence_pack`.

use crate::payloads::stored_search::{
    StoredConflictSet, StoredEvidenceRequirements, StoredRetrievalModelFingerprint,
};
use crate::payloads::stored_search_trace::StoredSearchStopReason;
use maestria_domain::{
    ClaimCoverageStatusRecord, ClaimEvidenceCoverageRecord, CorpusSnapshotId, EvidenceId,
    EvidencePackCompressionRecord, EvidencePackMetadataRecord, IndexGenerationId, QueryId,
    SearchTraceId,
};
use serde::{Deserialize, Serialize};

pub(crate) use super::stored_evidence_pack_coverage::{
    StoredEvidenceFreshnessRecord, StoredSourceIndependenceRecord,
};
pub(crate) use super::stored_evidence_pack_reproducibility::StoredEvidencePackReproducibilityRecord;

/// Wire mirror of `maestria_domain::ClaimCoverageStatusRecord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredClaimCoverageStatusRecord {
    Supported,
    Partial,
    Missing,
    Conflicted,
}

impl StoredClaimCoverageStatusRecord {
    pub(crate) fn from_domain(status: ClaimCoverageStatusRecord) -> Self {
        match status {
            ClaimCoverageStatusRecord::Supported => Self::Supported,
            ClaimCoverageStatusRecord::Partial => Self::Partial,
            ClaimCoverageStatusRecord::Missing => Self::Missing,
            ClaimCoverageStatusRecord::Conflicted => Self::Conflicted,
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ClaimCoverageStatusRecord, maestria_ports::PortError> {
        Ok(match self {
            Self::Supported => ClaimCoverageStatusRecord::Supported,
            Self::Partial => ClaimCoverageStatusRecord::Partial,
            Self::Missing => ClaimCoverageStatusRecord::Missing,
            Self::Conflicted => ClaimCoverageStatusRecord::Conflicted,
        })
    }
}

/// Wire mirror of `maestria_domain::ClaimEvidenceCoverageRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredClaimEvidenceCoverageRecord {
    pub(crate) claim: String,
    pub(crate) evidence_ids: Vec<u64>,
    pub(crate) status: StoredClaimCoverageStatusRecord,
}

impl StoredClaimEvidenceCoverageRecord {
    pub(crate) fn from_domain(record: &ClaimEvidenceCoverageRecord) -> Self {
        Self {
            claim: record.claim.clone(),
            evidence_ids: record.evidence_ids.iter().map(|id| id.value()).collect(),
            status: StoredClaimCoverageStatusRecord::from_domain(record.status),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<ClaimEvidenceCoverageRecord, maestria_ports::PortError> {
        Ok(ClaimEvidenceCoverageRecord {
            claim: self.claim,
            evidence_ids: self.evidence_ids.into_iter().map(EvidenceId::new).collect(),
            status: self.status.try_into_domain()?,
        })
    }
}

/// Wire mirror of `maestria_domain::EvidencePackCompressionRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredEvidencePackCompressionRecord {
    Verbatim {
        evidence_ids: Vec<u64>,
    },
    Compressed {
        source_evidence_ids: Vec<u64>,
        retained_evidence_ids: Vec<u64>,
        selector: String,
    },
}

impl StoredEvidencePackCompressionRecord {
    pub(crate) fn from_domain(record: &EvidencePackCompressionRecord) -> Self {
        match record {
            EvidencePackCompressionRecord::Verbatim { evidence_ids } => Self::Verbatim {
                evidence_ids: evidence_ids.iter().map(|id| id.value()).collect(),
            },
            EvidencePackCompressionRecord::Compressed {
                source_evidence_ids,
                retained_evidence_ids,
                selector,
            } => Self::Compressed {
                source_evidence_ids: source_evidence_ids.iter().map(|id| id.value()).collect(),
                retained_evidence_ids: retained_evidence_ids.iter().map(|id| id.value()).collect(),
                selector: selector.clone(),
            },
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<EvidencePackCompressionRecord, maestria_ports::PortError> {
        Ok(match self {
            Self::Verbatim { evidence_ids } => EvidencePackCompressionRecord::Verbatim {
                evidence_ids: evidence_ids.into_iter().map(EvidenceId::new).collect(),
            },
            Self::Compressed {
                source_evidence_ids,
                retained_evidence_ids,
                selector,
            } => EvidencePackCompressionRecord::Compressed {
                source_evidence_ids: source_evidence_ids
                    .into_iter()
                    .map(EvidenceId::new)
                    .collect(),
                retained_evidence_ids: retained_evidence_ids
                    .into_iter()
                    .map(EvidenceId::new)
                    .collect(),
                selector,
            },
        })
    }
}

/// Wire mirror of `maestria_domain::EvidencePackMetadataRecord`. Identifier
/// fields are flattened to their raw `u64` values; shared domain types
/// (`RetrievalModelFingerprint`, `EvidenceRequirements`, `ConflictSet`,
/// `FreshnessStatus`, `SearchStopReason`) delegate to their own stored
/// mirrors in `stored_search` / `stored_search_trace`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredEvidencePackMetadataRecord {
    pub(crate) query_id: u64,
    pub(crate) search_trace: Option<u64>,
    pub(crate) corpus_snapshot: u64,
    pub(crate) index_generation: u64,
    pub(crate) fingerprint: StoredRetrievalModelFingerprint,
    pub(crate) policy_fingerprint: Option<String>,
    pub(crate) claims_required: Vec<String>,
    pub(crate) requirements: StoredEvidenceRequirements,
    pub(crate) claim_coverage: Vec<StoredClaimEvidenceCoverageRecord>,
    pub(crate) source_independence: Vec<StoredSourceIndependenceRecord>,
    pub(crate) card_count: usize,
    pub(crate) distinct_sources: usize,
    pub(crate) distinct_documents: usize,
    pub(crate) distinct_sections: usize,
    pub(crate) primary_sources_verified: bool,
    pub(crate) freshness: Vec<StoredEvidenceFreshnessRecord>,
    pub(crate) conflicts: Vec<StoredConflictSet>,
    pub(crate) counterevidence: Vec<u64>,
    pub(crate) missing_evidence: Vec<String>,
    pub(crate) compression: StoredEvidencePackCompressionRecord,
    pub(crate) stop_reason: StoredSearchStopReason,
    pub(crate) reproducibility: StoredEvidencePackReproducibilityRecord,
}

impl StoredEvidencePackMetadataRecord {
    pub(crate) fn from_domain(record: &EvidencePackMetadataRecord) -> Self {
        Self {
            query_id: record.query_id.value(),
            search_trace: record.search_trace.map(|id| id.value()),
            corpus_snapshot: record.corpus_snapshot.value(),
            index_generation: record.index_generation.value(),
            fingerprint: StoredRetrievalModelFingerprint::from_domain(&record.fingerprint),
            policy_fingerprint: record.policy_fingerprint.clone(),
            claims_required: record.claims_required.clone(),
            requirements: StoredEvidenceRequirements::from_domain(&record.requirements),
            claim_coverage: record
                .claim_coverage
                .iter()
                .map(StoredClaimEvidenceCoverageRecord::from_domain)
                .collect(),
            source_independence: record
                .source_independence
                .iter()
                .map(StoredSourceIndependenceRecord::from_domain)
                .collect(),
            card_count: record.card_count,
            distinct_sources: record.distinct_sources,
            distinct_documents: record.distinct_documents,
            distinct_sections: record.distinct_sections,
            primary_sources_verified: record.primary_sources_verified,
            freshness: record
                .freshness
                .iter()
                .map(StoredEvidenceFreshnessRecord::from_domain)
                .collect(),
            conflicts: record
                .conflicts
                .iter()
                .map(StoredConflictSet::from_domain)
                .collect(),
            counterevidence: record.counterevidence.iter().map(|id| id.value()).collect(),
            missing_evidence: record.missing_evidence.clone(),
            compression: StoredEvidencePackCompressionRecord::from_domain(&record.compression),
            stop_reason: StoredSearchStopReason::from_domain(&record.stop_reason),
            reproducibility: StoredEvidencePackReproducibilityRecord::from_domain(
                &record.reproducibility,
            ),
        }
    }

    pub(crate) fn try_into_domain(
        self,
    ) -> Result<EvidencePackMetadataRecord, maestria_ports::PortError> {
        Ok(EvidencePackMetadataRecord {
            query_id: QueryId::new(self.query_id),
            search_trace: self.search_trace.map(SearchTraceId::new),
            corpus_snapshot: CorpusSnapshotId::new(self.corpus_snapshot),
            index_generation: IndexGenerationId::new(self.index_generation),
            fingerprint: self.fingerprint.try_into_domain()?,
            policy_fingerprint: self.policy_fingerprint,
            claims_required: self.claims_required,
            requirements: self.requirements.try_into_domain()?,
            claim_coverage: self
                .claim_coverage
                .into_iter()
                .map(StoredClaimEvidenceCoverageRecord::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
            source_independence: self
                .source_independence
                .into_iter()
                .map(StoredSourceIndependenceRecord::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
            card_count: self.card_count,
            distinct_sources: self.distinct_sources,
            distinct_documents: self.distinct_documents,
            distinct_sections: self.distinct_sections,
            primary_sources_verified: self.primary_sources_verified,
            freshness: self
                .freshness
                .into_iter()
                .map(StoredEvidenceFreshnessRecord::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
            conflicts: self
                .conflicts
                .into_iter()
                .map(StoredConflictSet::try_into_domain)
                .collect::<Result<Vec<_>, _>>()?,
            counterevidence: self
                .counterevidence
                .into_iter()
                .map(EvidenceId::new)
                .collect(),
            missing_evidence: self.missing_evidence,
            compression: self.compression.try_into_domain()?,
            stop_reason: self.stop_reason.try_into_domain()?,
            reproducibility: self.reproducibility.try_into_domain()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_domain::{
        ClaimCoverageStatusRecord, ClaimEvidenceCoverageRecord, ConflictSet, ConflictSetId,
        DuplicateClusterId, EvidenceFreshnessRecord, EvidencePackCompressionRecord,
        EvidencePackReplayKeyRecord, EvidencePackReproducibilityRecord, EvidenceRequirements,
        FreshnessStatus, RetrievalModelFingerprint, SearchStopReason, SourceIndependenceRecord,
    };

    fn record() -> Result<EvidencePackMetadataRecord, Box<dyn std::error::Error>> {
        Ok(EvidencePackMetadataRecord {
            query_id: QueryId::new(1),
            search_trace: Some(SearchTraceId::new(2)),
            corpus_snapshot: CorpusSnapshotId::new(3),
            index_generation: IndexGenerationId::new(4),
            fingerprint: RetrievalModelFingerprint::new("fingerprint-v1".to_string())?,
            policy_fingerprint: Some("policy-v1".to_string()),
            claims_required: vec!["claim-1".to_string()],
            requirements: EvidenceRequirements {
                require_primary_sources: true,
                minimum_corroboration: 2,
                required_claims: vec!["claim-1".to_string()],
                required_subquestions: vec![],
                minimum_sources: 3,
                minimum_documents: 2,
                minimum_sections: 1,
            },
            claim_coverage: vec![ClaimEvidenceCoverageRecord {
                claim: "claim-1".to_string(),
                evidence_ids: vec![EvidenceId::new(5)],
                status: ClaimCoverageStatusRecord::Supported,
            }],
            source_independence: vec![SourceIndependenceRecord {
                source_key: "example.com".to_string(),
                evidence_ids: vec![EvidenceId::new(5)],
                duplicate_cluster: Some(DuplicateClusterId::new(6)),
            }],
            card_count: 7,
            distinct_sources: 8,
            distinct_documents: 9,
            distinct_sections: 10,
            primary_sources_verified: true,
            freshness: vec![EvidenceFreshnessRecord {
                evidence_id: EvidenceId::new(5),
                status: FreshnessStatus::UpToDate,
            }],
            conflicts: vec![ConflictSet {
                id: ConflictSetId::new(11),
                candidates: vec![],
            }],
            counterevidence: vec![EvidenceId::new(12)],
            missing_evidence: vec!["missing-1".to_string()],
            compression: EvidencePackCompressionRecord::Compressed {
                source_evidence_ids: vec![EvidenceId::new(5)],
                retained_evidence_ids: vec![EvidenceId::new(13)],
                selector: "top-10".to_string(),
            },
            stop_reason: SearchStopReason::ResultsLimit,
            reproducibility: EvidencePackReproducibilityRecord::LiveNonReproducible {
                reason: "live run".to_string(),
            },
        })
    }

    #[test]
    fn metadata_record_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = record()?;
        let stored = StoredEvidencePackMetadataRecord::from_domain(&original);
        let json = serde_json::to_string(&stored)?;
        let decoded = serde_json::from_str::<StoredEvidencePackMetadataRecord>(&json)?;
        let restored = decoded.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn frozen_reproducibility_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let original = EvidencePackMetadataRecord {
            reproducibility: EvidencePackReproducibilityRecord::Frozen(
                EvidencePackReplayKeyRecord {
                    trace: SearchTraceId::new(2),
                    corpus_snapshot: CorpusSnapshotId::new(3),
                    index_generation: IndexGenerationId::new(4),
                    fingerprint: RetrievalModelFingerprint::new("fingerprint-v1".to_string())?,
                    policy_fingerprint: "policy-v1".to_string(),
                },
            ),
            ..record()?
        };
        let stored = StoredEvidencePackMetadataRecord::from_domain(&original);
        let restored = stored.try_into_domain()?;
        assert_eq!(restored, original);
        Ok(())
    }

    #[test]
    fn every_coverage_status_round_trips() -> Result<(), Box<dyn std::error::Error>> {
        for status in [
            ClaimCoverageStatusRecord::Supported,
            ClaimCoverageStatusRecord::Partial,
            ClaimCoverageStatusRecord::Missing,
            ClaimCoverageStatusRecord::Conflicted,
        ] {
            let stored = StoredClaimCoverageStatusRecord::from_domain(status);
            assert_eq!(stored.try_into_domain()?, status);
        }
        Ok(())
    }

    #[test]
    fn invalid_fingerprint_fails_domain_decode() -> Result<(), Box<dyn std::error::Error>> {
        // Rebuild with an invalid (empty) fingerprint string.
        let stored = StoredEvidencePackMetadataRecord::from_domain(&record()?);
        let mut json = serde_json::to_value(&stored)?;
        json.as_object_mut()
            .ok_or_else(|| "expected JSON object".to_string())?
            .insert("fingerprint".to_string(), serde_json::Value::from("   "));
        let decoded = serde_json::from_value::<StoredEvidencePackMetadataRecord>(json)?;
        assert!(decoded.try_into_domain().is_err());
        Ok(())
    }

    #[test]
    fn unknown_metadata_field_is_rejected_during_deserialization()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut value =
            serde_json::to_value(StoredEvidencePackMetadataRecord::from_domain(&record()?))?;
        value
            .as_object_mut()
            .ok_or_else(|| "expected JSON object".to_string())?
            .insert("extra".to_string(), serde_json::Value::from(1));
        assert!(serde_json::from_value::<StoredEvidencePackMetadataRecord>(value).is_err());
        Ok(())
    }
}
