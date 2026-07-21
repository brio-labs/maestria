use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use maestria_code_intel::{
    CodeQuery, REPOSITORY_CODE_PARSER_GENERATION, RepositoryCodeIndex, RepositoryFreshness,
    SymbolRecord,
};
use maestria_domain::{
    ArtifactVersionId, ContentRange, EvidenceCandidate, EvidenceId, EvidenceSpan, FreshnessStatus,
    IndexGenerationId, RetrievalReason, SearchLaneStatus, SecurityMetadata, SourceLocation,
    TrustLabel,
};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};

use crate::adapters::common::{generation_mismatch, one_based_rank};
use crate::adapters::score_provenance::specialized_score;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};
#[cfg(test)]
#[path = "code_intel_tests.rs"]
mod tests;

const CODE_INTEL_REPRESENTATION: &str = "repository_code_v1";

/// Dependencies required by the repository code intelligence adapter.
pub struct CodeIntelRetrieverParts {
    pub index: Arc<RepositoryCodeIndex>,
}

/// Deterministic repository-code retriever.
pub struct CodeIntelRetriever {
    index: Arc<RepositoryCodeIndex>,
    descriptor: RetrieverDescriptor,
    policy: RetrievalSecurityPolicy,
}

impl CodeIntelRetriever {
    pub fn new(
        parts: CodeIntelRetrieverParts,
        policy: RetrievalSecurityPolicy,
        generation: IndexGenerationId,
    ) -> Self {
        Self {
            index: parts.index,
            policy,
            descriptor: RetrieverDescriptor {
                id: "code_intel".to_string(),
                modality: "code".to_string(),
                representation: maestria_domain::RepresentationName::new(CODE_INTEL_REPRESENTATION),
                generation,
            },
        }
    }

    fn candidate_from_symbol(
        &self,
        symbol: &SymbolRecord,
        freshness: FreshnessStatus,
        rank: usize,
    ) -> Result<EvidenceCandidate, RetrievalError> {
        if !scan_secrets(&symbol.name).is_clean()
            || !scan_secrets(&symbol.qualified_name).is_clean()
            || !scan_secrets(&symbol.package).is_clean()
            || !scan_secrets(&symbol.target).is_clean()
        {
            return Err(RetrievalError::Internal(
                "repository code symbol rejected by secret scanner".to_string(),
            ));
        }
        if symbol.provenance.file_path.is_empty()
            || symbol.provenance.source_range.start_line == 0
            || symbol.provenance.source_range.end_line < symbol.provenance.source_range.start_line
            || symbol.provenance.source_range.end_line == 0
        {
            return Err(RetrievalError::Internal(
                "invalid source range in repository code symbol provenance".to_string(),
            ));
        }

        let artifact_version = artifact_version_id(symbol, &self.index.summary.repository_root);
        let source_span = EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: symbol.provenance.file_path.clone(),
                start_line: symbol.provenance.source_range.start_line as u32,
                end_line: symbol.provenance.source_range.end_line as u32,
            },
            ContentRange {
                start: symbol.provenance.source_range.start_line,
                end: symbol.provenance.source_range.end_line,
            },
        )
        .map_err(|error| RetrievalError::Internal(error.to_string()))?;

        Ok(EvidenceCandidate {
            evidence_id: evidence_id_from_symbol(symbol, artifact_version),
            artifact_version,
            source_span,
            scores: specialized_score(
                &self.descriptor,
                "repository_code",
                score_for_rank(rank),
                one_based_rank(rank),
                "repository_code_rank",
                BTreeMap::from([
                    (
                        "repository_root".to_string(),
                        self.index.summary.repository_root.clone(),
                    ),
                    (
                        "commit_sha".to_string(),
                        symbol.provenance.commit_sha.clone(),
                    ),
                    (
                        "worktree_identity".to_string(),
                        symbol.provenance.worktree_identity.clone(),
                    ),
                    (
                        "parser_generation".to_string(),
                        symbol.provenance.parser_generation.clone(),
                    ),
                    ("record_id".to_string(), symbol.record_id.clone()),
                    (
                        "source_path".to_string(),
                        symbol.provenance.file_path.clone(),
                    ),
                ]),
            )?,
            trust: TrustLabel::Unverified,
            freshness,
            duplicate_cluster: None,
            reasons: vec![RetrievalReason::SpecializedRetrieval {
                route: "repository_code".to_string(),
            }],
            coverage_keys: vec![
                format!("symbol:{}", symbol.record_id),
                format!("file:{}", symbol.provenance.file_path),
            ],
        })
    }

    fn freshness(&self) -> Result<FreshnessStatus, RetrievalError> {
        if self
            .index
            .is_stale_generation(REPOSITORY_CODE_PARSER_GENERATION)
        {
            return Ok(FreshnessStatus::Stale);
        }
        let freshness = self.index.freshness().map_err(|error| {
            RetrievalError::Internal(format!("repository code freshness check: {error}"))
        })?;
        Ok(freshness_status_to_domain(freshness))
    }
}

fn freshness_status_to_domain(freshness: RepositoryFreshness) -> FreshnessStatus {
    match freshness {
        RepositoryFreshness::Current { .. } => FreshnessStatus::UpToDate,
        RepositoryFreshness::Stale { .. } => FreshnessStatus::Stale,
    }
}

fn artifact_version_id(symbol: &SymbolRecord, repository_root: &str) -> ArtifactVersionId {
    let value = deterministic_hash(&[
        "artifact",
        repository_root,
        symbol.provenance.commit_sha.as_str(),
        symbol.provenance.worktree_identity.as_str(),
        symbol.provenance.parser_generation.as_str(),
        symbol.package.as_str(),
        symbol.provenance.file_path.as_str(),
    ]);
    ArtifactVersionId::new(value)
}

fn evidence_id_from_symbol(
    symbol: &SymbolRecord,
    artifact_version: ArtifactVersionId,
) -> EvidenceId {
    let value = deterministic_hash(&[
        "evidence",
        &artifact_version.value().to_string(),
        symbol.provenance.commit_sha.as_str(),
        symbol.provenance.worktree_identity.as_str(),
        symbol.provenance.parser_generation.as_str(),
        symbol.record_id.as_str(),
        symbol.package.as_str(),
        symbol.target.as_str(),
        symbol.qualified_name.as_str(),
    ]);
    EvidenceId::new(value)
}

fn score_for_rank(rank: usize) -> u32 {
    let offset = rank.min(100_000);
    1_000_000_u32.saturating_sub((offset as u32).saturating_mul(10))
}

fn deterministic_hash(parts: &[&str]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn symbol_pattern(query: &str) -> String {
    let mut segments = query.split('`');
    let _prefix = segments.next();
    segments
        .next()
        .filter(|segment| !segment.trim().is_empty())
        .map_or_else(|| query.to_string(), ToString::to_string)
}

#[async_trait]
impl CandidateRetriever for CodeIntelRetriever {
    fn descriptor(&self) -> crate::types::RetrieverDescriptor {
        self.descriptor.clone()
    }

    async fn retrieve(&self, request: CandidateRequest) -> Result<CandidateBatch, RetrievalError> {
        if request.expected_generation != self.descriptor.generation {
            return Err(generation_mismatch(
                request.expected_generation,
                self.descriptor.generation,
            ));
        }
        if !matches!(request.plan.scope, maestria_domain::CorpusScope::Global) {
            return Ok(CandidateBatch {
                descriptor: self.descriptor.clone(),
                query: request.query.q,
                candidates: Vec::new(),
                status: SearchLaneStatus::Empty,
                generation: Some(self.descriptor.generation),
                bytes_read: 0,
            });
        }
        if self.policy.evaluate(&SecurityMetadata::default()) != RetrievalDecision::Allowed {
            return Err(RetrievalError::Internal(
                "repository code retrieval denied by security policy".to_string(),
            ));
        }
        if !scan_secrets(&request.query.q).is_clean() {
            return Err(RetrievalError::Internal(
                "code query rejected by secret scanner".to_string(),
            ));
        }
        let freshness = self.freshness()?;
        let request_limit = request.query.limit.saturating_add(request.query.offset);
        let query = CodeQuery::Symbol {
            pattern: symbol_pattern(&request.query.q),
        };
        let hits = self
            .index
            .query(query, request_limit)
            .records
            .into_iter()
            .skip(request.query.offset)
            .take(request.query.limit)
            .collect::<Vec<_>>();

        let mut bytes_read = 0_u64;
        let mut candidates = Vec::with_capacity(hits.len());
        for (rank, symbol) in hits.into_iter().enumerate() {
            let candidate = self.candidate_from_symbol(&symbol, freshness.clone(), rank)?;
            bytes_read = bytes_read.saturating_add(
                candidate
                    .source_span
                    .range()
                    .end
                    .saturating_sub(candidate.source_span.range().start) as u64,
            );
            candidates.push(candidate);
        }
        let status = if candidates.is_empty() {
            SearchLaneStatus::Empty
        } else {
            SearchLaneStatus::Succeeded
        };

        Ok(CandidateBatch {
            descriptor: self.descriptor.clone(),
            query: request.query.q,
            candidates,
            status,
            generation: Some(self.descriptor.generation),
            bytes_read,
        })
    }
}
