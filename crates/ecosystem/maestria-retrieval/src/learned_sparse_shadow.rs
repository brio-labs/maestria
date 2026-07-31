#[path = "learned_sparse_shadow_execution.rs"]
mod learned_sparse_shadow_execution;
#[path = "learned_sparse_shadow_store.rs"]
mod learned_sparse_shadow_store;

pub(crate) use learned_sparse_shadow_execution::spawn_learned_sparse_shadow;
pub use learned_sparse_shadow_store::{LearnedSparseShadowStore, LearnedSparseShadowStoreError};
pub use maestria_ports::{
    LearnedSparseShadowCandidate, LearnedSparseShadowLane, LearnedSparseShadowLaneStatus,
    LearnedSparseShadowObservation, LearnedSparseShadowRoute,
};

use maestria_ports::{
    LEARNED_SPARSE_SHADOW_SCHEMA_VERSION, MAX_LEARNED_SPARSE_SHADOW_ERROR_CHARS,
    MAX_LEARNED_SPARSE_SHADOW_LATENCY_MS, MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS,
    MAX_LEARNED_SPARSE_SHADOW_RETRIEVERS,
};

const SHADOW_SCHEMA_VERSION: u16 = LEARNED_SPARSE_SHADOW_SCHEMA_VERSION;
const MAX_SHADOW_RETRIEVERS: usize = MAX_LEARNED_SPARSE_SHADOW_RETRIEVERS;
const MAX_SHADOW_ERROR_CHARS: usize = MAX_LEARNED_SPARSE_SHADOW_ERROR_CHARS;
const MAX_SHADOW_LATENCY_MS: u64 = MAX_LEARNED_SPARSE_SHADOW_LATENCY_MS;
const MAX_SHADOW_OBSERVATIONS: usize = MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS;
const DEFAULT_SHADOW_CAPACITY: usize = MAX_SHADOW_OBSERVATIONS;
