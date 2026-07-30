use std::{
    collections::{BTreeSet, VecDeque},
    sync::Arc,
};

use async_trait::async_trait;
use maestria_domain::{
    EvidenceCandidate, IndexStatus, Relation, RelationEndpoint, SearchExecution,
    SearchExecutionCompletion, SearchExecutionResource, SearchExecutionUsage,
};
use maestria_governance::{RetrievalDecision, scan_secrets};
use maestria_ports::{
    ArtifactRepository, BlobStore, ChunkRepository, EvidenceRepository, GraphIndex,
    GraphRelationQuery,
};

use super::SourceSnapshotVerifier;
use super::common::{candidate_from_records, one_based_rank, port_error};
use super::score_provenance::graph_score;
use crate::traits::ContextExpander;
use crate::types::{ContextExpansion, ExpansionPolicy, RankedCandidate, RetrievalError};
#[cfg(test)]
#[path = "secondary_tests.rs"]
mod tests;

/// Graph expansion owns only graph access; evidence selection stays governed by the caller.
/// Graph-backed context expansion that projects only verified artifact chunks.
pub struct HierarchyGraphExpander {
    graph: Arc<dyn GraphIndex + Send + Sync>,
    artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    chunks: Arc<dyn ChunkRepository + Send + Sync>,
    evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    verifier: SourceSnapshotVerifier,
}

pub struct HierarchyGraphExpanderParts {
    pub graph: Arc<dyn GraphIndex + Send + Sync>,
    pub artifacts: Arc<dyn ArtifactRepository + Send + Sync>,
    pub chunks: Arc<dyn ChunkRepository + Send + Sync>,
    pub evidence: Arc<dyn EvidenceRepository + Send + Sync>,
    pub blobs: Arc<dyn BlobStore + Send + Sync>,
}

impl HierarchyGraphExpander {
    pub fn new(parts: HierarchyGraphExpanderParts) -> Self {
        Self {
            graph: parts.graph,
            artifacts: parts.artifacts,
            chunks: parts.chunks,
            evidence: parts.evidence,
            verifier: SourceSnapshotVerifier::new(parts.blobs),
        }
    }
}

#[async_trait]
impl ContextExpander for HierarchyGraphExpander {
    fn expand(
        &self,
        candidates: &[RankedCandidate],
        policy: &ExpansionPolicy,
    ) -> Result<ContextExpansion, RetrievalError> {
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
        let relation_budget = policy.max_results.saturating_sub(expanded.len()).min(
            maestria_domain::saturating_usize(policy.execution_budget.max_work_units()),
        );
        let mut state = ExpansionState {
            expanded,
            seen_evidence,
            queue,
            visited_artifacts: BTreeSet::new(),
            next_graph_rank: 1,
            relation_visits_remaining: relation_budget,
            execution_usage: SearchExecutionUsage::default(),
            completion: SearchExecutionCompletion::Complete,
        };
        while let Some((endpoint, seed_rank, depth)) = state.queue.pop_front() {
            if depth >= policy.max_depth
                || state.expanded.len() >= policy.max_results
                || state.relation_visits_remaining == 0
            {
                continue;
            }
            self.expand_endpoint(endpoint, seed_rank, depth, policy, &mut state)?;
        }
        Ok(ContextExpansion {
            candidates: state.expanded,
            execution: SearchExecution::new(
                policy.execution_budget,
                state.execution_usage,
                state.completion,
            ),
        })
    }
}

struct ExpansionState {
    expanded: Vec<EvidenceCandidate>,
    seen_evidence: BTreeSet<maestria_domain::EvidenceId>,
    queue: VecDeque<(RelationEndpoint, u32, usize)>,
    visited_artifacts: BTreeSet<maestria_domain::ArtifactId>,
    next_graph_rank: u32,
    relation_visits_remaining: usize,
    execution_usage: SearchExecutionUsage,
    completion: SearchExecutionCompletion,
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
        if state.expanded.len() >= policy.max_results || state.relation_visits_remaining == 0 {
            return Ok(());
        }
        let remaining_work = policy
            .execution_budget
            .max_work_units()
            .saturating_sub(state.execution_usage.work_units);
        let relation_limit = state
            .relation_visits_remaining
            .min(maestria_domain::saturating_usize(remaining_work));
        let Some(query) =
            GraphRelationQuery::new(endpoint, maestria_domain::saturating_u64(relation_limit))
        else {
            state.completion =
                SearchExecutionCompletion::Exhausted(SearchExecutionResource::WorkUnits);
            return Ok(());
        };
        let page = self.graph.get_relations_for(query).map_err(port_error)?;
        if !page.complete {
            state.completion =
                SearchExecutionCompletion::Exhausted(SearchExecutionResource::WorkUnits);
        }
        for relation in page.relations {
            if state.expanded.len() >= policy.max_results || state.relation_visits_remaining == 0 {
                break;
            }
            state.relation_visits_remaining = state.relation_visits_remaining.saturating_sub(1);
            state.execution_usage.work_units = state.execution_usage.work_units.saturating_add(1);
            let Some(related) = self.related_artifact(
                endpoint,
                relation,
                depth,
                policy,
                &mut state.visited_artifacts,
            )?
            else {
                continue;
            };
            self.append_artifact_candidates(
                &related,
                seed_rank,
                policy.max_results,
                policy,
                state,
            )?;
            if related.depth < policy.max_depth {
                state
                    .queue
                    .push_back((related.endpoint, seed_rank, related.depth));
            }
        }
        if state.execution_usage.work_units >= policy.execution_budget.max_work_units() {
            state.completion =
                SearchExecutionCompletion::Exhausted(SearchExecutionResource::WorkUnits);
        }
        Ok(())
    }
}

/*
 * The relation budget is intentionally global to one expansion. A high-degree
 * endpoint can return many relations, and invalid relation evidence must not
 * allow that endpoint to consume unbounded repository work while the output
 * remains below its cap. Once this budget is exhausted, queued endpoints are
 * skipped explicitly and the caller retains the verified seed candidates.
 */
impl HierarchyGraphExpander {
    fn related_artifact(
        &self,
        endpoint: RelationEndpoint,
        relation: Relation,
        depth: usize,
        policy: &ExpansionPolicy,
        visited_artifacts: &mut BTreeSet<maestria_domain::ArtifactId>,
    ) -> Result<Option<RelatedArtifact>, RetrievalError> {
        if policy.authorization.evaluate(&relation.security) != RetrievalDecision::Allowed {
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
        let Some(relation_artifact) = self
            .artifacts
            .get(relation_evidence.artifact_id)
            .map_err(port_error)?
        else {
            return Ok(None);
        };
        if policy.authorization.evaluate(&relation_artifact.security) != RetrievalDecision::Allowed
        {
            return Ok(None);
        }
        if policy.authorization.evaluate(&relation_evidence.security) != RetrievalDecision::Allowed
            || !scan_secrets(&relation_evidence.excerpt).is_clean()
            || self
                .verifier
                .verify(&relation_evidence, &relation_artifact)
                .is_err()
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
            || policy.authorization.evaluate(&artifact.security) != RetrievalDecision::Allowed
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
}

impl HierarchyGraphExpander {
    fn append_artifact_candidates(
        &self,
        related: &RelatedArtifact,
        seed_rank: u32,
        max_results: usize,
        policy: &ExpansionPolicy,
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
            let evidence_id = maestria_domain::evidence_id_for(chunk.artifact_id, chunk.order);
            if state.seen_evidence.contains(&evidence_id) {
                continue;
            }
            let Some(evidence) = self.evidence.get(evidence_id).map_err(port_error)? else {
                continue;
            };
            if policy.authorization.evaluate(&evidence.security) != RetrievalDecision::Allowed
                || !scan_secrets(&chunk.text).is_clean()
                || !scan_secrets(&evidence.excerpt).is_clean()
                || self.verifier.verify(&evidence, &related.artifact).is_err()
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
