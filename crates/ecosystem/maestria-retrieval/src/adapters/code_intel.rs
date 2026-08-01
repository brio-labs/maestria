use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use maestria_code_intel::{
    CodeQuery, QueryResult, REPOSITORY_CODE_PARSER_GENERATION, RepositoryCodeIndex,
    RepositoryFreshness,
};
use maestria_domain::{
    ContentRange, EvidenceCandidate, EvidenceKind, EvidenceSpan, FreshnessStatus,
    IndexGenerationId, RetrievalReason, SearchExecution, SearchExecutionCompletion,
    SearchExecutionResource, SearchExecutionUsage, SearchLaneStatus, SourceLocation,
};
use maestria_governance::scan_secrets;

use crate::adapters::common::{generation_mismatch, one_based_rank};
use crate::adapters::score_provenance::specialized_score;
use crate::adapters::{AuthorizedCodeBinding, CodeIntelSecurityResolver, trust_label};
use crate::traits::CandidateRetriever;
use crate::types::{CandidateBatch, CandidateRequest, RetrievalError, RetrieverDescriptor};
#[cfg(test)]
#[path = "code_intel_tests.rs"]
mod tests;

const CODE_INTEL_REPRESENTATION: &str = "repository_code_v2";

/// Dependencies required by the repository code intelligence adapter.
pub struct CodeIntelRetrieverParts {
    pub index: Arc<RepositoryCodeIndex>,
    pub security: CodeIntelSecurityResolver,
}

/// Deterministic repository-code retriever.
pub struct CodeIntelRetriever {
    index: Arc<RepositoryCodeIndex>,
    security: CodeIntelSecurityResolver,
    descriptor: RetrieverDescriptor,
}

impl CodeIntelRetriever {
    pub fn new(parts: CodeIntelRetrieverParts, generation: IndexGenerationId) -> Self {
        Self {
            index: parts.index,
            security: parts.security,
            descriptor: RetrieverDescriptor {
                id: "code_intel".to_string(),
                modality: "code".to_string(),
                representation: maestria_domain::RepresentationName::new(CODE_INTEL_REPRESENTATION),
                generation,
            },
        }
    }

    fn candidate_from_binding(
        &self,
        binding: AuthorizedCodeBinding,
        freshness: FreshnessStatus,
        rank: usize,
    ) -> Result<EvidenceCandidate, RetrievalError> {
        let symbol = &binding.symbol;
        if symbol.provenance.file_path.is_empty()
            || symbol.provenance.source_range.start_line == 0
            || symbol.provenance.source_range.end_line < symbol.provenance.source_range.start_line
            || symbol.provenance.source_range.end_line == 0
        {
            return Err(RetrievalError::Internal(
                "invalid source range in repository code symbol provenance".to_string(),
            ));
        }
        let EvidenceKind::FileSpan { path, .. } = &binding.evidence.kind else {
            return Err(RetrievalError::Internal(
                "authorized repository code evidence is not a file span".to_string(),
            ));
        };
        let source_span = EvidenceSpan::new(
            None,
            SourceLocation::File {
                path: path.clone(),
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
            evidence_id: binding.evidence.id,
            artifact_version: binding.artifact_version,
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
                    (
                        "content_hash".to_string(),
                        symbol.provenance.content_hash.clone(),
                    ),
                    ("record_id".to_string(), symbol.record_id.clone()),
                    (
                        "source_path".to_string(),
                        symbol.provenance.file_path.clone(),
                    ),
                ]),
            )?,
            trust: trust_label(&binding.security),
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
    fn materialize_candidates(
        &self,
        request: &CandidateRequest,
        query_result: QueryResult,
        authorized_bindings: Vec<AuthorizedCodeBinding>,
        freshness: FreshnessStatus,
    ) -> Result<
        (
            Vec<EvidenceCandidate>,
            SearchExecutionUsage,
            SearchExecutionCompletion,
        ),
        RetrievalError,
    > {
        let mut bindings = authorized_bindings;
        let mut bytes_read = 0_u64;
        let mut candidates = Vec::with_capacity(query_result.records.len());
        let mut completion = if query_result.summary.truncated {
            SearchExecutionCompletion::Exhausted(SearchExecutionResource::Candidates)
        } else {
            SearchExecutionCompletion::Complete
        };
        for (rank, symbol) in query_result
            .records
            .into_iter()
            .skip(request.query.offset)
            .take(request.query.limit)
            .enumerate()
        {
            let candidate_count = maestria_domain::saturating_u64(candidates.len());
            if candidate_count >= request.execution_budget.max_results() {
                completion = SearchExecutionCompletion::Exhausted(SearchExecutionResource::Results);
                break;
            }
            if candidate_count >= request.execution_budget.max_candidates() {
                completion =
                    SearchExecutionCompletion::Exhausted(SearchExecutionResource::Candidates);
                break;
            }
            if candidate_count >= request.execution_budget.max_work_units() {
                completion =
                    SearchExecutionCompletion::Exhausted(SearchExecutionResource::WorkUnits);
                break;
            }
            let Some(position) = bindings
                .iter()
                .position(|binding| binding.symbol.record_id == symbol.record_id)
            else {
                return Err(RetrievalError::Internal(format!(
                    "authorized repository code binding is missing for {}",
                    symbol.record_id
                )));
            };
            let binding = bindings.remove(position);
            let span_bytes = maestria_domain::saturating_u64(
                binding
                    .symbol
                    .provenance
                    .source_range
                    .end_line
                    .saturating_sub(binding.symbol.provenance.source_range.start_line),
            );
            if let Some(limit) = request.execution_budget.max_bytes_read()
                && span_bytes > limit.get().saturating_sub(bytes_read)
            {
                completion =
                    SearchExecutionCompletion::Exhausted(SearchExecutionResource::BytesRead);
                break;
            }
            let candidate = self.candidate_from_binding(binding, freshness.clone(), rank)?;
            bytes_read = bytes_read.saturating_add(span_bytes);
            candidates.push(candidate);
        }
        let usage = SearchExecutionUsage::new(
            maestria_domain::saturating_u64(candidates.len()),
            maestria_domain::saturating_u64(query_result.summary.scanned),
            maestria_domain::saturating_u64(query_result.summary.scanned),
            bytes_read,
        );
        Ok((candidates, usage, completion))
    }

    fn freshness(&self) -> Result<FreshnessStatus, RetrievalError> {
        if self
            .index
            .is_stale_generation(REPOSITORY_CODE_PARSER_GENERATION)
        {
            return Err(RetrievalError::Internal(
                "repository code parser generation is stale".to_string(),
            ));
        }
        match self.index.freshness().map_err(|error| {
            RetrievalError::Internal(format!("repository code freshness check: {error}"))
        })? {
            RepositoryFreshness::Current { .. } => Ok(FreshnessStatus::UpToDate),
            RepositoryFreshness::Stale { .. } => Err(RetrievalError::Internal(
                "repository code index is stale".to_string(),
            )),
        }
    }
}

fn score_for_rank(rank: usize) -> u32 {
    let offset = rank.min(100_000);
    1_000_000_u32.saturating_sub((offset as u32).saturating_mul(10))
}

fn symbol_pattern(query: &str) -> String {
    let mut segments = query.split('`');
    let _prefix = segments.next();
    segments
        .next()
        .filter(|segment| !segment.trim().is_empty())
        .map_or_else(|| query.to_string(), ToString::to_string)
}

fn retain_authorized_binding(
    bindings: &mut Vec<AuthorizedCodeBinding>,
    binding: AuthorizedCodeBinding,
    limit: usize,
) {
    if limit == 0 {
        return;
    }
    bindings.push(binding);
    bindings.sort_by(|left, right| {
        (
            left.symbol.provenance.file_path.as_str(),
            left.symbol.provenance.source_range.start_line,
            left.symbol.qualified_name.as_str(),
        )
            .cmp(&(
                right.symbol.provenance.file_path.as_str(),
                right.symbol.provenance.source_range.start_line,
                right.symbol.qualified_name.as_str(),
            ))
    });
    bindings.truncate(limit);
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
                query: request.query.q,
                descriptor: self.descriptor.clone(),
                candidates: Vec::new(),
                status: SearchLaneStatus::Empty,
                generation: Some(self.descriptor.generation),
                execution: SearchExecution::new(
                    request.execution_budget,
                    Default::default(),
                    SearchExecutionCompletion::Complete,
                ),
            });
        }
        let security = self.security.clone();
        if !scan_secrets(&request.query.q).is_clean() {
            return Err(RetrievalError::Internal(
                "code query rejected by secret scanner".to_string(),
            ));
        }
        let freshness = self.freshness()?;
        let query = CodeQuery::Symbol {
            pattern: symbol_pattern(&request.query.q),
        };
        let scan_limit = maestria_domain::saturating_usize(
            request
                .execution_budget
                .max_candidates()
                .min(request.execution_budget.max_work_units()),
        );
        let mut authorized_bindings = Vec::new();
        let query_result = self.index.query(
            query,
            scan_limit,
            |symbol| -> Result<bool, RetrievalError> {
                let Some(binding) = security.resolve(symbol, &request.authorization)? else {
                    return Ok(false);
                };
                retain_authorized_binding(&mut authorized_bindings, binding, scan_limit);
                Ok(true)
            },
        )?;
        let (candidates, usage, completion) =
            self.materialize_candidates(&request, query_result, authorized_bindings, freshness)?;
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
            execution: SearchExecution::new(request.execution_budget, usage, completion),
        })
    }
}
