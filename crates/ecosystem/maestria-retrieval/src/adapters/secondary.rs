use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, IndexStatus, Relation, RelationEndpoint, SearchOutcome, SearchStatus,
    SearchStopReason,
};
use maestria_governance::{RetrievalDecision, RetrievalSecurityPolicy, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, GraphIndex,
};

use super::common::{SourceSnapshotVerifier, candidate_from_records, one_based_rank, port_error};
use super::score_provenance::graph_score;
use crate::traits::{ContextExpander, RetrievalEvaluator};
use crate::types::{ExpansionPolicy, RankedCandidate, RetrievalError, RetrievalEvaluationReport};

/// Graph expansion owns only graph access; evidence selection stays governed by the caller.
/// Graph-backed context expansion that projects only verified artifact chunks.
pub struct HierarchyGraphExpander {
    graph: Arc<dyn GraphIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    verifier: SourceSnapshotVerifier,
    policy: RetrievalSecurityPolicy,
}

pub struct HierarchyGraphExpanderParts {
    pub graph: Arc<dyn GraphIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
}

impl HierarchyGraphExpander {
    pub fn new(parts: HierarchyGraphExpanderParts, policy: RetrievalSecurityPolicy) -> Self {
        Self {
            graph: parts.graph,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
            policy,
        }
    }

    pub fn related_artifact_relations(
        &self,
        artifact_id: maestria_domain::ArtifactId,
    ) -> Result<Vec<Relation>, RetrievalError> {
        self.graph
            .get_relations_for(RelationEndpoint::Artifact(artifact_id))
            .map_err(port_error)
    }
}

#[async_trait]
impl ContextExpander for HierarchyGraphExpander {
    fn expand(
        &self,
        candidates: &[RankedCandidate],
        policy: &ExpansionPolicy,
    ) -> Result<Vec<EvidenceCandidate>, RetrievalError> {
        let expanded = candidates
            .iter()
            .map(|candidate| candidate.candidate.clone())
            .collect::<Vec<_>>();
        let seen_evidence = expanded
            .iter()
            .map(|candidate| candidate.evidence_id)
            .collect::<BTreeSet<_>>();
        let queue = candidates
            .iter()
            .map(|candidate| {
                (
                    RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(
                        candidate.candidate.artifact_version.value(),
                    )),
                    one_based_rank(candidate.rank),
                    0_usize,
                )
            })
            .collect::<VecDeque<_>>();
        let mut state = ExpansionState {
            expanded,
            seen_evidence,
            queue,
            visited_artifacts: BTreeSet::new(),
            next_graph_rank: 1,
        };
        while let Some((endpoint, seed_rank, depth)) = state.queue.pop_front() {
            if depth >= policy.max_depth || state.expanded.len() >= policy.max_results {
                continue;
            }
            self.expand_endpoint(endpoint, seed_rank, depth, policy, &mut state)?;
        }
        Ok(state.expanded)
    }
}

struct ExpansionState {
    expanded: Vec<EvidenceCandidate>,
    seen_evidence: BTreeSet<maestria_domain::EvidenceId>,
    queue: VecDeque<(RelationEndpoint, u32, usize)>,
    visited_artifacts: BTreeSet<maestria_domain::ArtifactId>,
    next_graph_rank: u32,
}

struct RelatedArtifact {
    endpoint: RelationEndpoint,
    artifact: maestria_domain::Artifact,
    confidence_milli: u16,
    depth: usize,
}

impl HierarchyGraphExpander {
    fn expand_endpoint(
        &self,
        endpoint: RelationEndpoint,
        seed_rank: u32,
        depth: usize,
        policy: &ExpansionPolicy,
        state: &mut ExpansionState,
    ) -> Result<(), RetrievalError> {
        let relations = self.graph.get_relations_for(endpoint).map_err(port_error)?;
        for relation in relations {
            let Some(related) =
                self.related_artifact(endpoint, relation, depth, &mut state.visited_artifacts)?
            else {
                continue;
            };
            self.append_artifact_candidates(&related, seed_rank, policy.max_results, state)?;
            if related.depth < policy.max_depth {
                state
                    .queue
                    .push_back((related.endpoint, seed_rank, related.depth));
            }
        }
        Ok(())
    }

    fn related_artifact(
        &self,
        endpoint: RelationEndpoint,
        relation: Relation,
        depth: usize,
        visited_artifacts: &mut BTreeSet<maestria_domain::ArtifactId>,
    ) -> Result<Option<RelatedArtifact>, RetrievalError> {
        if self.policy.evaluate(&relation.security) != RetrievalDecision::Allowed {
            return Ok(None);
        }
        let Some(relation_evidence_id) = relation.evidence_id else {
            return Ok(None);
        };
        let Some(relation_evidence) = self
            .evidence
            .get(relation_evidence_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        if self.policy.evaluate(&relation_evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&relation_evidence.excerpt).is_clean()
            || self.verifier.verify(&relation_evidence).is_err()
        {
            return Ok(None);
        }
        let neighbor = if relation.source == endpoint {
            relation.target
        } else {
            relation.source
        };
        let RelationEndpoint::Artifact(artifact_id) = neighbor else {
            return Ok(None);
        };
        if !visited_artifacts.insert(artifact_id) {
            return Ok(None);
        }
        let Some(artifact) = self.artifacts.get(artifact_id).map_err(port_error)? else {
            return Ok(None);
        };
        if artifact.index_status != IndexStatus::Indexed
            || self.policy.evaluate(&artifact.security) != RetrievalDecision::Allowed
        {
            return Ok(None);
        }
        Ok(Some(RelatedArtifact {
            endpoint: neighbor,
            artifact,
            confidence_milli: relation.confidence_milli,
            depth: depth.saturating_add(1),
        }))
    }

    fn append_artifact_candidates(
        &self,
        related: &RelatedArtifact,
        seed_rank: u32,
        max_results: usize,
        state: &mut ExpansionState,
    ) -> Result<(), RetrievalError> {
        let mut chunks = self
            .chunks
            .list_for_artifact(related.artifact.id)
            .map_err(port_error)?;
        chunks.sort_by_key(|chunk| (chunk.order, chunk.id));
        let raw_score = graph_rank_score(seed_rank, related.depth, related.confidence_milli);
        for chunk in chunks {
            if state.expanded.len() >= max_results {
                break;
            }
            if !scan_secrets(&chunk.text).is_clean() {
                continue;
            }
            let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
            if state.seen_evidence.contains(&evidence_id) {
                continue;
            }
            let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
                continue;
            };
            if self.policy.evaluate(&evidence.security) != RetrievalDecision::Allowed
                || !scan_secrets(&evidence.excerpt).is_clean()
                || self.verifier.verify(&evidence).is_err()
            {
                continue;
            }
            let raw_rank = state.next_graph_rank;
            let candidate = candidate_from_records(
                related.artifact.id,
                &chunk.source_span,
                &evidence,
                chunk.node_id,
                graph_score(
                    raw_score,
                    raw_rank,
                    seed_rank,
                    related.depth,
                    related.confidence_milli,
                )?,
                vec![maestria_domain::RetrievalReason::GraphTraversal],
            )?;
            state.next_graph_rank = state.next_graph_rank.saturating_add(1);
            state.seen_evidence.insert(evidence_id);
            state.expanded.push(candidate);
        }
        Ok(())
    }
}

fn graph_rank_score(seed_rank: u32, depth: usize, confidence_milli: u16) -> u32 {
    let depth = match u32::try_from(depth) {
        Ok(depth) => depth.max(1),
        Err(_) => u32::MAX,
    };
    1_000_000_u32
        .saturating_mul(u32::from(confidence_milli))
        .saturating_div(1_000)
        .saturating_div(seed_rank.max(1))
        .saturating_div(depth)
}

/// Evaluates already-filtered evidence candidates into a durable outcome.
pub struct EvidenceOutcomeEvaluator {
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
}

impl EvidenceOutcomeEvaluator {
    pub fn new(evidence: Arc<dyn EvidenceRepository + Send + Sync>) -> Self {
        Self { evidence }
    }
}

#[async_trait]
impl RetrievalEvaluator for EvidenceOutcomeEvaluator {
    async fn evaluate(
        &self,
        experiment: crate::types::RetrievalExperiment,
    ) -> Result<RetrievalEvaluationReport, RetrievalError> {
        let evidence = experiment.candidates;
        let status = if evidence.is_empty() {
            SearchStatus::NoEvidenceFound
        } else {
            SearchStatus::Answerable
        };
        let coverage = maestria_domain::EvidenceCoverage {
            percent_covered: if evidence.is_empty() { 0 } else { 100 },
            gaps_identified: Vec::new(),
            required_claims: experiment
                .plan
                .evidence_requirements
                .required_claims
                .clone(),
            required_subquestions: experiment
                .plan
                .evidence_requirements
                .required_subquestions
                .clone(),
            distinct_sources: evidence.len(),
            distinct_documents: evidence.len(),
            distinct_sections: evidence.len(),
            candidate_coverage_keys: evidence
                .iter()
                .flat_map(|candidate| candidate.coverage_keys.clone())
                .collect(),
        };
        let stop_reason = if evidence.is_empty() {
            SearchStopReason::NoEvidence
        } else if evidence.len() >= experiment.plan.stop_conditions.max_results as usize {
            SearchStopReason::ResultsLimit
        } else {
            SearchStopReason::EvidenceComplete
        };
        let diversity = maestria_domain::SearchTraceDiversity {
            distinct_sources: coverage.distinct_sources,
            distinct_documents: coverage.distinct_documents,
            distinct_sections: coverage.distinct_sections,
            required_claims: coverage.required_claims.clone(),
            required_subquestions: coverage.required_subquestions.clone(),
            covered_keys: coverage.candidate_coverage_keys.clone(),
            stop_reason: stop_reason.clone(),
            candidates: evidence
                .iter()
                .enumerate()
                .map(
                    |(rank, candidate)| maestria_domain::SearchTraceDiversityCandidate {
                        candidate_id: candidate.evidence_id,
                        original_rank: rank,
                        selected_rank: Some(rank),
                        duplicate_cluster: candidate.duplicate_cluster,
                        marginal_coverage: 100,
                        coverage_keys: candidate.coverage_keys.clone(),
                    },
                )
                .collect(),
        };
        let mut trace = maestria_domain::SearchTrace::from_plan(
            &experiment.plan,
            vec!["evidence".to_string()],
            &evidence,
            Vec::new(),
            None,
            Vec::new(),
            stop_reason.clone(),
        );
        trace.diversity = Some(diversity);
        let outcome = SearchOutcome {
            trace: trace.deterministic_id(),
            trace_data: Some(Box::new(trace)),
            fingerprint: experiment.plan.fingerprint.clone(),
            index_generation: experiment.plan.index_generation,
            status,
            evidence,
            coverage,
            conflicts: Vec::new(),
        };
        outcome.verify_compatibility(&experiment.plan)?;
        let _ = &self.evidence;
        Ok(RetrievalEvaluationReport {
            evaluated_candidates: outcome.evidence.len(),
            outcome,
        })
    }
}
