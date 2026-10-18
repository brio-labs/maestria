use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use sillage_search_tantivy::RamMemoryReservation;

use super::{
    ingest::SessionStores,
    retained::{PersistedGeneration, RetainedSources},
    source_admission::SourceLease,
};
use crate::SearchRuntime;

pub(super) struct SessionResourceState {
    resources: Mutex<Option<SessionResources>>,
}

pub(super) struct SessionResources {
    pub(super) runtime: Arc<SearchRuntime>,
    pub(super) _runtime_admission: Arc<Vec<RamMemoryReservation>>,
    pub(super) stores: Arc<SessionStores>,
    pub(super) retained_sources: Arc<RetainedSources>,
    pub(super) retained_generation: Option<PersistedGeneration>,
    pub(super) _source_lease: Option<SourceLease>,
}

#[derive(Clone)]
pub(super) struct SessionResourceSnapshot {
    pub(super) runtime: Arc<SearchRuntime>,
    pub(super) _runtime_admission: Arc<Vec<RamMemoryReservation>>,
    pub(super) stores: Arc<SessionStores>,
    pub(super) retained_sources: Arc<RetainedSources>,
}

impl SessionResourceState {
    pub(super) fn new(resources: SessionResources) -> Self {
        Self {
            resources: Mutex::new(Some(resources)),
        }
    }

    pub(super) fn snapshot(&self) -> Result<SessionResourceSnapshot> {
        let resources = self
            .resources
            .lock()
            .map_err(|_| super::SessionLifecycleFailure::SessionResourceState)?;
        let resources = resources
            .as_ref()
            .ok_or_else(|| anyhow!("session source has been retired"))?;
        Ok(SessionResourceSnapshot {
            runtime: resources.runtime.clone(),
            _runtime_admission: resources._runtime_admission.clone(),
            stores: resources.stores.clone(),
            retained_sources: resources.retained_sources.clone(),
        })
    }

    pub(super) fn with_resources<T>(
        &self,
        operation: impl FnOnce(&SessionResources) -> Result<T>,
    ) -> Result<T> {
        let resources = self
            .resources
            .lock()
            .map_err(|_| super::SessionLifecycleFailure::SessionResourceState)?;
        let resources = resources
            .as_ref()
            .ok_or_else(|| anyhow!("session source has been retired"))?;
        operation(resources)
    }

    pub(super) fn commit_retention(&self) -> Result<()> {
        let mut resources = self
            .resources
            .lock()
            .map_err(|_| super::SessionLifecycleFailure::SessionResourceState)?;
        let resources = resources
            .as_mut()
            .ok_or_else(|| anyhow!("session source has been retired"))?;
        if let Some(mut generation) = resources.retained_generation.take() {
            generation.commit();
        }
        Ok(())
    }

    pub(super) fn clear(&self) -> Result<()> {
        match self.resources.lock() {
            Ok(mut resources) => drop(resources.take()),
            Err(poisoned) => {
                drop(poisoned.into_inner().take());
                return Err(super::SessionLifecycleFailure::SessionResourceState.into());
            }
        }
        Ok(())
    }
}
