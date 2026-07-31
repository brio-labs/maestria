use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use maestria_ports::{
    LearnedSparseObservationRepository, LearnedSparseShadowObservation,
    MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS,
};
use thiserror::Error;

use super::DEFAULT_SHADOW_CAPACITY;

/// Errors raised while creating or replaying the bounded shadow observation buffer.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LearnedSparseShadowStoreError {
    #[error("learned-sparse shadow capacity must be positive")]
    InvalidCapacity,
    #[error("invalid learned-sparse shadow observation: {0}")]
    InvalidObservation(String),
    #[error("learned-sparse shadow serialization failed: {0}")]
    Serialization(String),
    #[error("learned-sparse shadow persistence failed: {0}")]
    Persistence(String),
}

/// In-memory runtime buffer for bounded, serializable shadow observations.
#[derive(Clone)]
pub struct LearnedSparseShadowStore {
    capacity: usize,
    observations: Arc<Mutex<VecDeque<LearnedSparseShadowObservation>>>,
    persistence_errors: Arc<Mutex<VecDeque<String>>>,
    repository: Option<Arc<dyn LearnedSparseObservationRepository>>,
}

impl Default for LearnedSparseShadowStore {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_SHADOW_CAPACITY,
            observations: Arc::new(Mutex::new(VecDeque::new())),
            persistence_errors: Arc::new(Mutex::new(VecDeque::new())),
            repository: None,
        }
    }
}

impl LearnedSparseShadowStore {
    pub fn new(capacity: usize) -> Result<Self, LearnedSparseShadowStoreError> {
        if capacity == 0 || capacity > MAX_LEARNED_SPARSE_SHADOW_OBSERVATIONS {
            return Err(LearnedSparseShadowStoreError::InvalidCapacity);
        }
        Ok(Self {
            capacity,
            observations: Arc::new(Mutex::new(VecDeque::new())),
            persistence_errors: Arc::new(Mutex::new(VecDeque::new())),
            repository: None,
        })
    }

    pub fn with_repository(
        mut self,
        repository: Arc<dyn LearnedSparseObservationRepository>,
    ) -> Self {
        self.repository = Some(repository);
        if let Err(error) = self.restore_from_repository() {
            self.record_persistence_error(error.to_string());
        }
        self
    }

    pub fn restore_from_repository(&self) -> Result<(), LearnedSparseShadowStoreError> {
        let Some(repository) = &self.repository else {
            return Ok(());
        };
        let limit = self.capacity_limit()?;
        let observations = repository
            .scan_observations(limit)
            .map_err(|error| LearnedSparseShadowStoreError::Persistence(error.to_string()))?;
        self.replace_memory(bounded_observations(observations, self.capacity))
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

    pub fn persistence_errors(&self) -> Vec<String> {
        let errors = match self.persistence_errors.lock() {
            Ok(errors) => errors,
            Err(poisoned) => poisoned.into_inner(),
        };
        errors.iter().cloned().collect()
    }

    pub fn export_json(&self) -> Result<String, LearnedSparseShadowStoreError> {
        let observations = match &self.repository {
            Some(repository) => repository
                .scan_observations(self.capacity_limit()?)
                .map_err(|error| LearnedSparseShadowStoreError::Persistence(error.to_string()))?,
            None => self.snapshot(),
        };
        serde_json::to_string(&observations)
            .map_err(|error| LearnedSparseShadowStoreError::Serialization(error.to_string()))
    }

    pub fn replace_from_json(&self, input: &str) -> Result<(), LearnedSparseShadowStoreError> {
        let observations: Vec<LearnedSparseShadowObservation> = serde_json::from_str(input)
            .map_err(|error| LearnedSparseShadowStoreError::Serialization(error.to_string()))?;
        for observation in &observations {
            validate_observation(observation)?;
        }
        let observations = bounded_observations(observations, self.capacity);
        if let Some(repository) = &self.repository {
            repository
                .replace_observations(observations.clone())
                .map_err(|error| LearnedSparseShadowStoreError::Persistence(error.to_string()))?;
        }
        self.replace_memory(observations)
    }

    fn capacity_limit(&self) -> Result<NonZeroUsize, LearnedSparseShadowStoreError> {
        NonZeroUsize::new(self.capacity).ok_or(LearnedSparseShadowStoreError::InvalidCapacity)
    }

    fn replace_memory(
        &self,
        observations: Vec<LearnedSparseShadowObservation>,
    ) -> Result<(), LearnedSparseShadowStoreError> {
        let mut current = match self.observations.lock() {
            Ok(observations) => observations,
            Err(poisoned) => poisoned.into_inner(),
        };
        current.clear();
        current.extend(observations);
        Ok(())
    }

    pub(crate) fn record(&self, observation: LearnedSparseShadowObservation) {
        if let Err(error) = validate_observation(&observation) {
            self.record_persistence_error(error.to_string());
            return;
        }
        let capacity = match self.capacity_limit() {
            Ok(capacity) => capacity,
            Err(error) => {
                self.record_persistence_error(error.to_string());
                return;
            }
        };
        if let Some(repository) = &self.repository {
            if let Err(error) = repository.append_observation(observation.clone()) {
                self.record_persistence_error(error.to_string());
            } else if let Err(error) = repository.prune_observations(capacity) {
                self.record_persistence_error(error.to_string());
            }
        }
        let mut observations = match self.observations.lock() {
            Ok(observations) => observations,
            Err(poisoned) => poisoned.into_inner(),
        };
        while observations.len() >= self.capacity {
            let _discarded = observations.pop_front();
        }
        observations.push_back(observation);
    }

    fn record_persistence_error(&self, error: String) {
        let mut errors = match self.persistence_errors.lock() {
            Ok(errors) => errors,
            Err(poisoned) => poisoned.into_inner(),
        };
        while errors.len() >= self.capacity {
            let _discarded = errors.pop_front();
        }
        errors.push_back(super::learned_sparse_shadow_execution::bounded_error(
            &error,
        ));
    }
}

fn bounded_observations(
    observations: Vec<LearnedSparseShadowObservation>,
    capacity: usize,
) -> Vec<LearnedSparseShadowObservation> {
    observations
        .into_iter()
        .rev()
        .take(capacity)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn validate_observation(
    observation: &LearnedSparseShadowObservation,
) -> Result<(), LearnedSparseShadowStoreError> {
    observation
        .validate()
        .map_err(|error| LearnedSparseShadowStoreError::InvalidObservation(error.to_string()))
}
