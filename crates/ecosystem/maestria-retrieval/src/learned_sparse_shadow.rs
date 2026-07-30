use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maestria_domain::{
    ArtifactVersionId, CorpusSnapshotId, EvidenceId, EvidenceSpan, IndexGenerationId,
    LearnedSparseReason, QueryId, RetrievalLaneScore, RetrievalReason, RetrievalScoreKind,
    RetrievalScoreSet, SearchLaneStatus, SearchPlan,
};
use maestria_ports::SearchQuery;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::learned_sparse_benchmark::LearnedSparseQueryClass;
use crate::traits::CandidateRetriever;
use crate::types::{CandidateRequest, RetrieverDescriptor};

const SHADOW_SCHEMA_VERSION: u16 = 3;
const MAX_SHADOW_RETRIEVERS: usize = 8;
const MAX_SHADOW_ERROR_CHARS: usize = 512;
const MAX_SHADOW_LATENCY_MS: u64 = 5_000;
const DEFAULT_SHADOW_CAPACITY: usize = 256;

/// One bounded learned-sparse candidate observed outside the served result path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseShadowCandidate {
    pub evidence_id: EvidenceId,
    pub artifact_version: ArtifactVersionId,
    pub source_span: EvidenceSpan,
    pub lane_rank: u32,
    pub score: RetrievalLaneScore,
    pub reason: LearnedSparseReason,
}

/// Non-serving execution status for one sparse retriever lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LearnedSparseShadowLaneStatus {
    Succeeded,
    Empty,
    Failed { error: String },
    TimedOut,
}

/// Bounded observation for one learned-sparse retriever invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseShadowLane {
    pub retriever_id: String,
    pub representation: maestria_domain::RepresentationName,
    pub generation: IndexGenerationId,
    #[serde(default)]
    pub namespace: Option<maestria_domain::SparseNamespace>,
    pub status: LearnedSparseShadowLaneStatus,
    pub candidates: Vec<LearnedSparseShadowCandidate>,
}

/// A non-serving learned-sparse observation that cannot alter the served outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSparseShadowObservation {
    pub schema_version: u16,
    pub query_id: QueryId,
    pub query_class: LearnedSparseQueryClass,
    pub corpus_snapshot: CorpusSnapshotId,
    pub index_generation: IndexGenerationId,
    pub elapsed_ms: u64,
    pub lanes: Vec<LearnedSparseShadowLane>,
}

/// Errors raised while creating or replaying the bounded shadow observation buffer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LearnedSparseShadowStoreError {
    #[error("learned-sparse shadow capacity must be positive")]
    InvalidCapacity,
    #[error("invalid learned-sparse shadow observation: {0}")]
    InvalidObservation(String),
    #[error("learned-sparse shadow serialization failed: {0}")]
    Serialization(String),
}

/// In-memory runtime buffer for bounded, serializable shadow observations.
#[derive(Clone)]
pub struct LearnedSparseShadowStore {
    capacity: usize,
    observations: Arc<Mutex<VecDeque<LearnedSparseShadowObservation>>>,
}

/// Owned handle for one non-serving shadow execution.
///
/// Dropping the handle aborts the provider task. A successfully completed
/// search may call [`Self::release`] to preserve the existing fire-and-forget
/// observation semantics.
pub(crate) struct LearnedSparseShadowTask {
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for LearnedSparseShadowTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl LearnedSparseShadowTask {
    pub(crate) fn release(mut self) {
        let _released = self.handle.take();
    }
}

impl Default for LearnedSparseShadowStore {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_SHADOW_CAPACITY,
            observations: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

impl LearnedSparseShadowStore {
    pub fn new(capacity: usize) -> Result<Self, LearnedSparseShadowStoreError> {
        if capacity == 0 {
            return Err(LearnedSparseShadowStoreError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            observations: Arc::new(Mutex::new(VecDeque::new())),
        })
    }

    pub fn snapshot(&self) -> Vec<LearnedSparseShadowObservation> {
        let observations = match self.observations.lock() {
            Ok(observations) => observations,
            Err(poisoned) => poisoned.into_inner(),
        };
        observations.iter().cloned().collect()
    }

    pub fn drain(&self) -> Vec<LearnedSparseShadowObservation> {
        let mut observations = match self.observations.lock() {
            Ok(observations) => observations,
            Err(poisoned) => poisoned.into_inner(),
        };
        observations.drain(..).collect()
    }

    pub fn export_json(&self) -> Result<String, LearnedSparseShadowStoreError> {
        serde_json::to_string(&self.snapshot())
            .map_err(|error| LearnedSparseShadowStoreError::Serialization(error.to_string()))
    }

    pub fn replace_from_json(&self, input: &str) -> Result<(), LearnedSparseShadowStoreError> {
        let observations: Vec<LearnedSparseShadowObservation> = serde_json::from_str(input)
            .map_err(|error| LearnedSparseShadowStoreError::Serialization(error.to_string()))?;
        for observation in &observations {
            validate_observation(observation)?;
        }
        let mut current = match self.observations.lock() {
            Ok(observations) => observations,
            Err(poisoned) => poisoned.into_inner(),
        };
        current.clear();
        current.extend(
            observations
                .into_iter()
                .rev()
                .take(self.capacity)
                .collect::<Vec<_>>()
                .into_iter()
                .rev(),
        );
        Ok(())
    }

    fn record(&self, observation: LearnedSparseShadowObservation) {
        let mut observations = match self.observations.lock() {
            Ok(observations) => observations,
            Err(poisoned) => poisoned.into_inner(),
        };
        while observations.len() >= self.capacity {
            let _discarded = observations.pop_front();
        }
        observations.push_back(observation);
    }
}

pub(crate) fn spawn_learned_sparse_shadow(
    retrievers: Vec<Arc<dyn CandidateRetriever>>,
    plan: SearchPlan,
    authorization: maestria_governance::RetrievalAuthorizationContext,
    store: LearnedSparseShadowStore,
) -> Option<LearnedSparseShadowTask> {
    let retrievers = retrievers
        .into_iter()
        .take(MAX_SHADOW_RETRIEVERS)
        .map(|retriever| {
            let descriptor = retriever.descriptor();
            let namespace = retriever.sparse_namespace();
            (retriever, descriptor, namespace)
        })
        .collect::<Vec<_>>();
    if retrievers.is_empty() {
        return None;
    }
    Some(LearnedSparseShadowTask {
        handle: Some(tokio::spawn(async move {
            let observation = run_shadow(retrievers, plan, authorization).await;
            store.record(observation);
        })),
    })
}
async fn run_shadow(
    retrievers: Vec<(
        Arc<dyn CandidateRetriever>,
        RetrieverDescriptor,
        Option<maestria_domain::SparseNamespace>,
    )>,
    plan: SearchPlan,
    authorization: maestria_governance::RetrievalAuthorizationContext,
) -> LearnedSparseShadowObservation {
    let started = tokio::time::Instant::now();
    let timeout_ms = u64::from(plan.budgets.max_latency_ms()).clamp(1, MAX_SHADOW_LATENCY_MS);
    let shadow_retrievers = retrievers.clone();
    let lanes = match plan.execution_budget() {
        Ok(execution_budget) => match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            collect_shadow_lanes(shadow_retrievers, &plan, &authorization, execution_budget),
        )
        .await
        {
            Ok(lanes) => lanes,
            Err(_) => retrievers
                .iter()
                .map(|(_, descriptor, namespace)| LearnedSparseShadowLane {
                    retriever_id: descriptor.id.clone(),
                    representation: descriptor.representation.clone(),
                    generation: descriptor.generation,
                    namespace: namespace.clone(),
                    status: LearnedSparseShadowLaneStatus::TimedOut,
                    candidates: Vec::new(),
                })
                .collect(),
        },
        Err(error) => {
            let error = bounded_error(&format!("invalid shadow execution budget: {error}"));
            retrievers
                .into_iter()
                .map(|(_, descriptor, namespace)| failed_lane(descriptor, namespace, &error))
                .collect()
        }
    };
    LearnedSparseShadowObservation {
        schema_version: SHADOW_SCHEMA_VERSION,
        query_id: plan.query_id,
        query_class: LearnedSparseQueryClass::classify(&plan.original_query),
        corpus_snapshot: plan.corpus_snapshot,
        index_generation: plan.index_generation,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        lanes,
    }
}
fn partition_allowance(total: u64, lanes: usize, lane: usize) -> u64 {
    let lanes = lanes.max(1) as u64;
    let base = total / lanes;
    base + if (lane as u64) < (total % lanes) {
        1
    } else {
        0
    }
}

fn shadow_lane_budget(
    global: maestria_domain::SearchExecutionBudget,
    lanes: usize,
    lane: usize,
) -> Option<maestria_domain::SearchExecutionBudget> {
    let max_bytes = match global.max_bytes_read() {
        Some(limit) => Some(std::num::NonZeroU64::new(partition_allowance(
            limit.get(),
            lanes,
            lane,
        ))?),
        None => None,
    };
    maestria_domain::SearchExecutionBudget::with_byte_limit(
        partition_allowance(global.max_results(), lanes, lane),
        partition_allowance(global.max_candidates(), lanes, lane),
        partition_allowance(global.max_work_units(), lanes, lane),
        max_bytes,
    )
    .ok()
}

async fn collect_shadow_lanes(
    retrievers: Vec<(
        Arc<dyn CandidateRetriever>,
        RetrieverDescriptor,
        Option<maestria_domain::SparseNamespace>,
    )>,
    plan: &SearchPlan,
    authorization: &maestria_governance::RetrievalAuthorizationContext,
    execution_budget: maestria_domain::SearchExecutionBudget,
) -> Vec<LearnedSparseShadowLane> {
    let mut lanes = Vec::with_capacity(retrievers.len());
    let lane_count = retrievers.len().max(1);
    for (lane_index, (retriever, descriptor, namespace)) in retrievers.into_iter().enumerate() {
        let Some(execution_budget) = shadow_lane_budget(execution_budget, lane_count, lane_index)
        else {
            lanes.push(failed_lane(
                descriptor,
                namespace,
                "shadow execution budget exhausted before lane allocation",
            ));
            continue;
        };
        let query_limit = maestria_domain::saturating_usize(execution_budget.max_results());
        let max_candidates = maestria_domain::saturating_usize(execution_budget.max_candidates());
        let max_contributions =
            maestria_domain::saturating_usize(execution_budget.max_work_units());
        let request = CandidateRequest {
            plan: plan.clone(),
            query: SearchQuery {
                q: plan.original_query.clone(),
                limit: query_limit,
                offset: 0,
                execution_budget,
            },
            execution_budget,
            expected_generation: descriptor.generation,
            authorization: authorization.clone(),
        };
        let lane = match retriever.retrieve(request).await {
            Ok(batch)
                if batch.execution.budget != execution_budget
                    || batch.execution.usage.results
                        < maestria_domain::saturating_u64(batch.candidates.len())
                    || batch.execution.usage.candidates
                        < maestria_domain::saturating_u64(batch.candidates.len())
                    || batch.execution.usage.work_units
                        < maestria_domain::saturating_u64(batch.candidates.len()) =>
            {
                failed_lane(
                    descriptor,
                    namespace.clone(),
                    "shadow lane returned invalid execution metadata",
                )
            }
            Ok(batch) if batch.generation != Some(descriptor.generation) => failed_lane(
                descriptor,
                namespace.clone(),
                "shadow lane returned an incompatible generation",
            ),
            Ok(batch) => lane_from_batch(
                descriptor,
                namespace,
                batch,
                max_candidates,
                max_contributions,
            ),
            Err(error) => failed_lane(descriptor, namespace, &error.to_string()),
        };
        lanes.push(lane);
    }
    lanes
}

fn lane_from_batch(
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    batch: crate::types::CandidateBatch,
    max_candidates: usize,
    max_contributions: usize,
) -> LearnedSparseShadowLane {
    let candidates = batch
        .candidates
        .iter()
        .take(max_candidates)
        .enumerate()
        .filter_map(|(rank, candidate)| shadow_candidate(candidate, rank, max_contributions))
        .collect::<Vec<_>>();
    let status = match batch.status {
        SearchLaneStatus::Succeeded if candidates.is_empty() => {
            LearnedSparseShadowLaneStatus::Failed {
                error: "sparse lane returned candidates without sparse provenance".to_string(),
            }
        }
        SearchLaneStatus::Succeeded => LearnedSparseShadowLaneStatus::Succeeded,
        SearchLaneStatus::Empty => LearnedSparseShadowLaneStatus::Empty,
        SearchLaneStatus::Failed { error } => LearnedSparseShadowLaneStatus::Failed {
            error: bounded_error(&error),
        },
    };
    LearnedSparseShadowLane {
        retriever_id: descriptor.id,
        representation: descriptor.representation,
        generation: descriptor.generation,
        namespace,
        status,
        candidates,
    }
}

fn shadow_candidate(
    candidate: &maestria_domain::EvidenceCandidate,
    rank: usize,
    max_contributions: usize,
) -> Option<LearnedSparseShadowCandidate> {
    let score = candidate
        .scores
        .lane(&RetrievalScoreKind::LearnedSparse)?
        .clone();
    candidate.reasons.iter().find_map(|reason| {
        let RetrievalReason::LearnedSparse(reason) = reason else {
            return None;
        };
        let mut reason = reason.as_ref().clone();
        reason.contributions.truncate(max_contributions);
        Some(LearnedSparseShadowCandidate {
            evidence_id: candidate.evidence_id,
            artifact_version: candidate.artifact_version,
            source_span: candidate.source_span.clone(),
            lane_rank: match u32::try_from(rank.saturating_add(1)) {
                Ok(value) => value,
                Err(e) => {
                    let _ = e;
                    u32::MAX
                }
            },
            score: score.clone(),
            reason,
        })
    })
}

fn failed_lane(
    descriptor: RetrieverDescriptor,
    namespace: Option<maestria_domain::SparseNamespace>,
    error: &str,
) -> LearnedSparseShadowLane {
    LearnedSparseShadowLane {
        retriever_id: descriptor.id,
        representation: descriptor.representation,
        generation: descriptor.generation,
        namespace,
        status: LearnedSparseShadowLaneStatus::Failed {
            error: bounded_error(error),
        },
        candidates: Vec::new(),
    }
}

fn bounded_error(error: &str) -> String {
    error.chars().take(MAX_SHADOW_ERROR_CHARS).collect()
}

fn validate_observation(
    observation: &LearnedSparseShadowObservation,
) -> Result<(), LearnedSparseShadowStoreError> {
    if observation.schema_version != SHADOW_SCHEMA_VERSION {
        return Err(LearnedSparseShadowStoreError::InvalidObservation(
            "unsupported schema version".to_string(),
        ));
    }
    if observation.elapsed_ms > MAX_SHADOW_LATENCY_MS {
        return Err(LearnedSparseShadowStoreError::InvalidObservation(
            "observation latency exceeds the bounded limit".to_string(),
        ));
    }
    if observation.lanes.len() > MAX_SHADOW_RETRIEVERS {
        return Err(LearnedSparseShadowStoreError::InvalidObservation(
            "retriever lane cap exceeded".to_string(),
        ));
    }
    for lane in &observation.lanes {
        if lane.retriever_id.trim().is_empty()
            || lane
                .namespace
                .as_ref()
                .is_none_or(|namespace| namespace.validate().is_err())
            || lane.candidates.iter().any(|candidate| {
                candidate.score.score_kind != RetrievalScoreKind::LearnedSparse
                    || RetrievalScoreSet::single(candidate.score.clone()).is_err()
            })
        {
            return Err(LearnedSparseShadowStoreError::InvalidObservation(
                "lane identity or candidate provenance is invalid".to_string(),
            ));
        }
        if (matches!(lane.status, LearnedSparseShadowLaneStatus::Succeeded)
            && lane.candidates.is_empty())
            || (matches!(lane.status, LearnedSparseShadowLaneStatus::Empty)
                && !lane.candidates.is_empty())
            || lane
                .candidates
                .iter()
                .any(|candidate| candidate.lane_rank == 0)
        {
            return Err(LearnedSparseShadowStoreError::InvalidObservation(
                "lane status or candidate rank is invalid".to_string(),
            ));
        }
        if let LearnedSparseShadowLaneStatus::Failed { error } = &lane.status
            && error.chars().count() > MAX_SHADOW_ERROR_CHARS
        {
            return Err(LearnedSparseShadowStoreError::InvalidObservation(
                "failure reason exceeds the bounded error cap".to_string(),
            ));
        }
    }
    Ok(())
}
