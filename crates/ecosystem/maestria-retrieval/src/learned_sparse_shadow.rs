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

const SHADOW_SCHEMA_VERSION: u16 = 2;
const MAX_SHADOW_RETRIEVERS: usize = 8;
const MAX_SHADOW_CANDIDATES_PER_LANE: usize = 20;
const MAX_SHADOW_CONTRIBUTIONS: usize = 16;
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
    pub status: LearnedSparseShadowLaneStatus,
    pub candidates: Vec<LearnedSparseShadowCandidate>,
}

/// A detached learned-sparse observation that cannot alter the served outcome.
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
    store: LearnedSparseShadowStore,
) {
    let retrievers = retrievers
        .into_iter()
        .take(MAX_SHADOW_RETRIEVERS)
        .map(|retriever| {
            let descriptor = retriever.descriptor();
            (retriever, descriptor)
        })
        .collect::<Vec<_>>();
    if retrievers.is_empty() {
        return;
    }
    let handle = tokio::spawn(async move {
        let observation = run_shadow(retrievers, plan).await;
        store.record(observation);
    });
    drop(handle);
}

async fn run_shadow(
    retrievers: Vec<(Arc<dyn CandidateRetriever>, RetrieverDescriptor)>,
    plan: SearchPlan,
) -> LearnedSparseShadowObservation {
    let started = tokio::time::Instant::now();
    let timeout_ms = u64::from(plan.budgets.max_latency_ms()).clamp(1, MAX_SHADOW_LATENCY_MS);
    let descriptors = retrievers
        .iter()
        .map(|(_, descriptor)| descriptor.clone())
        .collect::<Vec<_>>();
    let lanes = match tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        collect_shadow_lanes(retrievers, &plan),
    )
    .await
    {
        Ok(lanes) => lanes,
        Err(_) => descriptors
            .into_iter()
            .map(|descriptor| LearnedSparseShadowLane {
                retriever_id: descriptor.id,
                representation: descriptor.representation,
                generation: descriptor.generation,
                status: LearnedSparseShadowLaneStatus::TimedOut,
                candidates: Vec::new(),
            })
            .collect(),
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

async fn collect_shadow_lanes(
    retrievers: Vec<(Arc<dyn CandidateRetriever>, RetrieverDescriptor)>,
    plan: &SearchPlan,
) -> Vec<LearnedSparseShadowLane> {
    let mut lanes = Vec::with_capacity(retrievers.len());
    for (retriever, descriptor) in retrievers {
        let request = CandidateRequest {
            plan: plan.clone(),
            query: SearchQuery {
                q: plan.original_query.clone(),
                limit: plan
                    .stop_conditions
                    .max_results
                    .min(MAX_SHADOW_CANDIDATES_PER_LANE as u32) as usize,
                offset: 0,
            },
            expected_generation: descriptor.generation,
        };
        let lane = match retriever.retrieve(request).await {
            Ok(batch) if batch.generation != Some(descriptor.generation) => failed_lane(
                descriptor,
                "shadow lane returned an incompatible generation",
            ),
            Ok(batch) => lane_from_batch(descriptor, batch),
            Err(error) => failed_lane(descriptor, &error.to_string()),
        };
        lanes.push(lane);
    }
    lanes
}

fn lane_from_batch(
    descriptor: RetrieverDescriptor,
    batch: crate::types::CandidateBatch,
) -> LearnedSparseShadowLane {
    let candidates = batch
        .candidates
        .iter()
        .take(MAX_SHADOW_CANDIDATES_PER_LANE)
        .enumerate()
        .filter_map(|(rank, candidate)| shadow_candidate(candidate, rank))
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
        status,
        candidates,
    }
}

fn shadow_candidate(
    candidate: &maestria_domain::EvidenceCandidate,
    rank: usize,
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
        reason.contributions.truncate(MAX_SHADOW_CONTRIBUTIONS);
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

fn failed_lane(descriptor: RetrieverDescriptor, error: &str) -> LearnedSparseShadowLane {
    LearnedSparseShadowLane {
        retriever_id: descriptor.id,
        representation: descriptor.representation,
        generation: descriptor.generation,
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
    if observation.lanes.len() > MAX_SHADOW_RETRIEVERS {
        return Err(LearnedSparseShadowStoreError::InvalidObservation(
            "retriever lane cap exceeded".to_string(),
        ));
    }
    for lane in &observation.lanes {
        if lane.retriever_id.trim().is_empty()
            || lane.candidates.len() > MAX_SHADOW_CANDIDATES_PER_LANE
            || lane.candidates.iter().any(|candidate| {
                candidate.reason.contributions.len() > MAX_SHADOW_CONTRIBUTIONS
                    || candidate.score.score_kind != RetrievalScoreKind::LearnedSparse
                    || RetrievalScoreSet::single(candidate.score.clone()).is_err()
            })
        {
            return Err(LearnedSparseShadowStoreError::InvalidObservation(
                "lane identity or bounded candidate provenance is invalid".to_string(),
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
