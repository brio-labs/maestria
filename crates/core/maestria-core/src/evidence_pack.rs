use maestria_domain::{
    ConflictSet, CorpusSnapshotId, DuplicateClusterId, EvidenceId, EvidenceKind,
    EvidenceRequirements, FreshnessStatus, IndexGenerationId, RetrievalModelFingerprint,
    SearchPlan, SearchStopReason, SearchTraceId,
};

use super::SourceGroundedSearchHit;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimCoverageStatus {
    Supported,
    Partial,
    Missing,
    Conflicted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimEvidenceCoverage {
    pub claim: String,
    pub evidence_ids: Vec<EvidenceId>,
    pub status: ClaimCoverageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIndependence {
    pub source_key: String,
    pub evidence_ids: Vec<EvidenceId>,
    pub duplicate_cluster: Option<DuplicateClusterId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceFreshness {
    pub evidence_id: EvidenceId,
    pub status: FreshnessStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidencePackCompression {
    Verbatim {
        evidence_ids: Vec<EvidenceId>,
    },
    Compressed {
        source_evidence_ids: Vec<EvidenceId>,
        retained_evidence_ids: Vec<EvidenceId>,
        selector: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePackReplayKey {
    pub trace: SearchTraceId,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub fingerprint: RetrievalModelFingerprint,
    pub policy_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidencePackReproducibility {
    Frozen(EvidencePackReplayKey),
    LiveNonReproducible { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidencePackMetadata {
    pub query_id: maestria_domain::QueryId,
    pub search_trace: Option<SearchTraceId>,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub fingerprint: RetrievalModelFingerprint,
    pub policy_fingerprint: Option<String>,
    pub claims_required: Vec<String>,
    pub requirements: EvidenceRequirements,
    pub claim_coverage: Vec<ClaimEvidenceCoverage>,
    pub source_independence: Vec<SourceIndependence>,
    pub card_count: usize,
    pub distinct_sources: usize,
    pub distinct_documents: usize,
    pub distinct_sections: usize,
    pub primary_sources_verified: bool,
    pub freshness: Vec<EvidenceFreshness>,
    pub conflicts: Vec<ConflictSet>,
    pub counterevidence: Vec<EvidenceId>,
    pub missing_evidence: Vec<String>,
    pub compression: EvidencePackCompression,
    pub stop_reason: SearchStopReason,
    pub reproducibility: EvidencePackReproducibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidencePackError {
    InvalidFreeze(String),
    InvalidCompression(String),
    InvalidCoverage(String),
    FrozenMutation(String),
    NotReproducible,
    ReplayIdentityMismatch,
    UnmaterializedEvidence(String),
}
impl std::fmt::Display for EvidencePackError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidFreeze(message) => {
                write!(formatter, "invalid evidence pack freeze: {message}")
            }
            Self::InvalidCompression(message) => {
                write!(formatter, "invalid evidence pack compression: {message}")
            }
            Self::InvalidCoverage(message) => {
                write!(formatter, "invalid evidence pack coverage: {message}")
            }
            Self::FrozenMutation(message) => {
                write!(formatter, "frozen evidence pack mutation: {message}")
            }
            Self::NotReproducible => formatter.write_str("evidence pack is not reproducible"),
            Self::ReplayIdentityMismatch => {
                formatter.write_str("evidence pack replay identity does not match")
            }
            Self::UnmaterializedEvidence(message) => {
                write!(formatter, "unmaterialized evidence in pack: {message}")
            }
        }
    }
}

impl std::error::Error for EvidencePackError {}

impl EvidencePackMetadata {
    pub fn from_plan(plan: &SearchPlan) -> Self {
        let mut seen_claims = std::collections::BTreeSet::new();
        let claims_required = plan
            .evidence_requirements
            .required_claims
            .iter()
            .chain(plan.evidence_requirements.required_subquestions.iter())
            .filter(|claim| seen_claims.insert(*claim))
            .cloned()
            .collect();
        Self {
            query_id: plan.query_id,
            search_trace: None,
            corpus_snapshot: plan.corpus_snapshot,
            index_generation: plan.index_generation,
            fingerprint: plan.fingerprint.clone(),
            policy_fingerprint: None,
            claims_required,
            requirements: plan.evidence_requirements.clone(),
            claim_coverage: Vec::new(),
            source_independence: Vec::new(),
            card_count: 0,
            distinct_sources: 0,
            distinct_documents: 0,
            distinct_sections: 0,
            primary_sources_verified: !plan.evidence_requirements.require_primary_sources,
            freshness: Vec::new(),
            conflicts: Vec::new(),
            counterevidence: Vec::new(),
            missing_evidence: Vec::new(),
            compression: EvidencePackCompression::Verbatim {
                evidence_ids: Vec::new(),
            },
            stop_reason: SearchStopReason::NoEvidence,
            reproducibility: EvidencePackReproducibility::LiveNonReproducible {
                reason: "live evidence has not been frozen".to_string(),
            },
        }
    }

    pub fn persistence_record(&self) -> maestria_domain::EvidencePackMetadataRecord {
        use maestria_domain::{
            ClaimCoverageStatusRecord, ClaimEvidenceCoverageRecord, EvidenceFreshnessRecord,
            EvidencePackCompressionRecord, EvidencePackReplayKeyRecord,
            EvidencePackReproducibilityRecord, SourceIndependenceRecord,
        };

        maestria_domain::EvidencePackMetadataRecord {
            query_id: self.query_id,
            search_trace: self.search_trace,
            corpus_snapshot: self.corpus_snapshot,
            index_generation: self.index_generation,
            fingerprint: self.fingerprint.clone(),
            policy_fingerprint: self.policy_fingerprint.clone(),
            claims_required: self.claims_required.clone(),
            requirements: self.requirements.clone(),
            claim_coverage: self
                .claim_coverage
                .iter()
                .map(|coverage| ClaimEvidenceCoverageRecord {
                    claim: coverage.claim.clone(),
                    evidence_ids: coverage.evidence_ids.clone(),
                    status: match coverage.status {
                        ClaimCoverageStatus::Supported => ClaimCoverageStatusRecord::Supported,
                        ClaimCoverageStatus::Partial => ClaimCoverageStatusRecord::Partial,
                        ClaimCoverageStatus::Missing => ClaimCoverageStatusRecord::Missing,
                        ClaimCoverageStatus::Conflicted => ClaimCoverageStatusRecord::Conflicted,
                    },
                })
                .collect(),
            source_independence: self
                .source_independence
                .iter()
                .map(|source| SourceIndependenceRecord {
                    source_key: source.source_key.clone(),
                    evidence_ids: source.evidence_ids.clone(),
                    duplicate_cluster: source.duplicate_cluster,
                })
                .collect(),
            card_count: self.card_count,
            distinct_sources: self.distinct_sources,
            distinct_documents: self.distinct_documents,
            distinct_sections: self.distinct_sections,
            primary_sources_verified: self.primary_sources_verified,
            freshness: self
                .freshness
                .iter()
                .map(|freshness| EvidenceFreshnessRecord {
                    evidence_id: freshness.evidence_id,
                    status: freshness.status.clone(),
                })
                .collect(),
            conflicts: self.conflicts.clone(),
            counterevidence: self.counterevidence.clone(),
            missing_evidence: self.missing_evidence.clone(),
            compression: match &self.compression {
                EvidencePackCompression::Verbatim { evidence_ids } => {
                    EvidencePackCompressionRecord::Verbatim {
                        evidence_ids: evidence_ids.clone(),
                    }
                }
                EvidencePackCompression::Compressed {
                    source_evidence_ids,
                    retained_evidence_ids,
                    selector,
                } => EvidencePackCompressionRecord::Compressed {
                    source_evidence_ids: source_evidence_ids.clone(),
                    retained_evidence_ids: retained_evidence_ids.clone(),
                    selector: selector.clone(),
                },
            },
            stop_reason: self.stop_reason.clone(),
            reproducibility: match &self.reproducibility {
                EvidencePackReproducibility::Frozen(key) => {
                    EvidencePackReproducibilityRecord::Frozen(EvidencePackReplayKeyRecord {
                        trace: key.trace,
                        corpus_snapshot: key.corpus_snapshot,
                        index_generation: key.index_generation,
                        fingerprint: key.fingerprint.clone(),
                        policy_fingerprint: key.policy_fingerprint.clone(),
                    })
                }
                EvidencePackReproducibility::LiveNonReproducible { reason } => {
                    EvidencePackReproducibilityRecord::LiveNonReproducible {
                        reason: reason.clone(),
                    }
                }
            },
        }
    }
    pub(crate) fn populate_from_chunks(
        &mut self,
        chunks: &[SourceGroundedSearchHit],
        evidence_ids: &[EvidenceId],
        card_count: usize,
    ) {
        use std::collections::{BTreeMap, BTreeSet};

        if matches!(self.compression, EvidencePackCompression::Compressed { .. }) {
            return;
        }
        self.card_count = card_count;
        self.freshness.clear();
        let chunk_ids = chunks
            .iter()
            .map(|hit| hit.evidence.id)
            .collect::<BTreeSet<_>>();
        let mut sources = BTreeMap::new();
        for hit in chunks {
            sources
                .entry(source_key(&hit.evidence.kind))
                .or_insert_with(Vec::new)
                .push(hit.evidence.id);
            self.freshness.push(EvidenceFreshness {
                evidence_id: hit.evidence.id,
                status: FreshnessStatus::Unknown,
            });
        }
        for evidence_id in evidence_ids {
            if !chunk_ids.contains(evidence_id) {
                self.freshness.push(EvidenceFreshness {
                    evidence_id: *evidence_id,
                    status: FreshnessStatus::Unknown,
                });
            }
        }
        self.claim_coverage = self
            .claims_required
            .iter()
            .map(|claim| ClaimEvidenceCoverage {
                claim: claim.clone(),
                evidence_ids: Vec::new(),
                status: ClaimCoverageStatus::Missing,
            })
            .collect();
        self.missing_evidence = self
            .claim_coverage
            .iter()
            .map(|coverage| coverage.claim.clone())
            .collect();
        self.source_independence = sources
            .into_iter()
            .map(|(source_key, evidence_ids)| SourceIndependence {
                source_key,
                evidence_ids,
                duplicate_cluster: None,
            })
            .collect();
        self.distinct_sources = self.source_independence.len();
        self.distinct_documents = chunks
            .iter()
            .map(|hit| hit.artifact.id)
            .collect::<BTreeSet<_>>()
            .len();
        self.distinct_sections = chunks
            .iter()
            .map(|hit| (hit.artifact.id, hit.chunk.node_id))
            .collect::<BTreeSet<_>>()
            .len();
        self.compression = EvidencePackCompression::Verbatim {
            evidence_ids: evidence_ids.to_vec(),
        };
        self.refresh_stop_reason();
    }

    pub fn set_conflicts(&mut self, conflicts: Vec<ConflictSet>, counterevidence: Vec<EvidenceId>) {
        self.conflicts = conflicts;
        self.counterevidence = counterevidence;
        self.refresh_stop_reason();
        if !self.conflicts.is_empty() {
            self.stop_reason = SearchStopReason::RequirementsUnmet;
        }
    }
    pub fn mark_primary_sources_verified(&mut self, verified: bool) {
        self.primary_sources_verified = verified;
        self.refresh_stop_reason();
    }
    pub(crate) fn refresh_stop_reason(&mut self) {
        let required_sources = self
            .requirements
            .minimum_sources
            .max(usize::from(self.requirements.minimum_corroboration));
        let card_results = usize::from(self.card_count > 0);
        let available_sources = self.distinct_sources.max(card_results);
        let available_documents = self.distinct_documents.max(card_results);
        let available_sections = self.distinct_sections.max(card_results);
        let no_evidence = self.compression_evidence_is_empty();
        let requirements_unmet = !self.missing_evidence.is_empty()
            || !self.conflicts.is_empty()
            || available_sources < required_sources
            || available_documents < self.requirements.minimum_documents
            || available_sections < self.requirements.minimum_sections
            || (self.requirements.require_primary_sources && !self.primary_sources_verified);
        let explicit_requirement_gap = !self.missing_evidence.is_empty()
            || !self.conflicts.is_empty()
            || (self.requirements.require_primary_sources && !self.primary_sources_verified);
        self.stop_reason = if no_evidence && !explicit_requirement_gap {
            SearchStopReason::NoEvidence
        } else if requirements_unmet {
            SearchStopReason::RequirementsUnmet
        } else {
            SearchStopReason::EvidenceComplete
        };
    }

    fn compression_evidence_is_empty(&self) -> bool {
        match &self.compression {
            EvidencePackCompression::Verbatim { evidence_ids } => {
                evidence_ids.is_empty() && self.card_count == 0
            }
            EvidencePackCompression::Compressed {
                retained_evidence_ids,
                ..
            } => retained_evidence_ids.is_empty() && self.card_count == 0,
        }
    }
}

fn source_key(kind: &EvidenceKind) -> String {
    match kind {
        EvidenceKind::FileSpan { path, .. } => format!("file:{path}"),
        EvidenceKind::PdfSpan { blob, .. } | EvidenceKind::PdfRegion { blob, .. } => {
            format!("blob:{}", blob.value())
        }
        EvidenceKind::WebSnapshot { url, .. } => format!("web:{url}"),
        EvidenceKind::CommandOutput { harness_run, .. }
        | EvidenceKind::TestResult { harness_run, .. }
        | EvidenceKind::Diff { harness_run, .. } => format!("run:{}", harness_run.value()),
        EvidenceKind::Validation { report_id } => format!("validation:{}", report_id.value()),
    }
}
