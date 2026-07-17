use crate::errors::DomainError;
use crate::ids::IndexGenerationId;
use crate::search::ContentHash;
use std::collections::BTreeMap;

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize, ::serde::Deserialize,
)]
pub struct RepresentationName(pub String);

impl RepresentationName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexFingerprint {
    pub provider: String,
    pub model: String,
    pub revision: String,
    pub artifact_hash: ContentHash,
    pub dimensions: u32,
    pub quantization: String,
    pub query_template_hash: String,
    pub document_template_hash: String,
    pub preprocessing_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub enum IndexLifecycle {
    Building,
    Evaluated,
    Shadow,
    Active,
    Retired,
    Collectable,
    Tombstoned,
}

impl IndexLifecycle {
    pub fn can_transition_to(&self, next: IndexLifecycle) -> bool {
        match (self, next) {
            // normal forward progression
            (IndexLifecycle::Building, IndexLifecycle::Evaluated) => true,
            (IndexLifecycle::Evaluated, IndexLifecycle::Shadow) => true,
            (IndexLifecycle::Shadow, IndexLifecycle::Active) => true,
            (IndexLifecycle::Active, IndexLifecycle::Retired) => true,
            (IndexLifecycle::Retired, IndexLifecycle::Collectable) => true,
            (IndexLifecycle::Collectable, IndexLifecycle::Tombstoned) => true,

            // rollback
            (IndexLifecycle::Retired, IndexLifecycle::Active) => true,

            // direct deletion from any state
            (_, IndexLifecycle::Tombstoned) if *self != IndexLifecycle::Tombstoned => true,

            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub struct IndexGeneration {
    pub id: IndexGenerationId,
    pub name: RepresentationName,
    pub corpus_snapshot: crate::ids::CorpusSnapshotId,
    pub fingerprint: IndexFingerprint,
    pub lifecycle: IndexLifecycle,
}

impl IndexGeneration {
    pub fn is_serveable(&self) -> bool {
        self.lifecycle == IndexLifecycle::Active
    }

    pub fn transition_to(&mut self, next: IndexLifecycle) -> Result<(), DomainError> {
        if !self.lifecycle.can_transition_to(next) {
            return Err(DomainError::InvalidGenerationTransition {
                id: self.id,
                from: self.lifecycle,
                to: next,
            });
        }
        self.lifecycle = next;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexGenerationRegistry {
    generations: BTreeMap<IndexGenerationId, IndexGeneration>,
    active_generations: BTreeMap<RepresentationName, IndexGenerationId>,
}

impl IndexGenerationRegistry {
    pub fn register(&mut self, generation: IndexGeneration) -> Result<(), DomainError> {
        if generation.lifecycle != IndexLifecycle::Building {
            return Err(DomainError::InvalidGenerationTransition {
                id: generation.id,
                from: generation.lifecycle,
                to: IndexLifecycle::Building,
            });
        }
        if self.generations.contains_key(&generation.id) {
            return Err(DomainError::DuplicateId {
                kind: "IndexGeneration",
                id: generation.id.0,
            });
        }
        self.generations.insert(generation.id, generation);
        Ok(())
    }

    pub fn get(&self, id: IndexGenerationId) -> Option<&IndexGeneration> {
        self.generations.get(&id)
    }

    pub fn get_active(&self, name: &RepresentationName) -> Option<&IndexGeneration> {
        self.active_generations
            .get(name)
            .and_then(|id| self.generations.get(id))
    }

    pub fn active_id(&self, name: &RepresentationName) -> Option<IndexGenerationId> {
        self.active_generations.get(name).copied()
    }

    pub fn is_serveable(&self, id: IndexGenerationId) -> bool {
        self.get(id).is_some_and(|generation| {
            generation.is_serveable() && self.active_id(&generation.name) == Some(id)
        })
    }

    pub fn is_empty(&self) -> bool {
        self.generations.is_empty()
    }
    /// Returns all registered generations in deterministic identifier order.
    ///
    /// The registry remains the owner of lifecycle and active-generation invariants;
    /// callers receive read-only views for diagnostics and projection reporting.
    pub fn iter(&self) -> impl Iterator<Item = &IndexGeneration> {
        self.generations.values()
    }

    pub fn transition_lifecycle(
        &mut self,
        id: IndexGenerationId,
        next: IndexLifecycle,
    ) -> Result<Option<IndexGenerationId>, DomainError> {
        let generation = self
            .generations
            .get(&id)
            .ok_or(DomainError::MissingIndexGeneration { id })?;
        let previous = generation.lifecycle;
        if !previous.can_transition_to(next) {
            return Err(DomainError::InvalidGenerationTransition {
                id,
                from: previous,
                to: next,
            });
        }
        let name = generation.name.clone();

        let previous_active = if next == IndexLifecycle::Active {
            self.active_generations
                .get(&name)
                .copied()
                .filter(|active_id| *active_id != id)
        } else {
            None
        };
        if let Some(old_id) = previous_active {
            let old = self
                .generations
                .get(&old_id)
                .ok_or(DomainError::MissingIndexGeneration { id: old_id })?;
            if !old.lifecycle.can_transition_to(IndexLifecycle::Retired) {
                return Err(DomainError::InvalidGenerationTransition {
                    id: old_id,
                    from: old.lifecycle,
                    to: IndexLifecycle::Retired,
                });
            }
        }
        if let Some(old_id) = previous_active {
            self.generations
                .get_mut(&old_id)
                .ok_or(DomainError::MissingIndexGeneration { id: old_id })?
                .transition_to(IndexLifecycle::Retired)?;
        }

        self.generations
            .get_mut(&id)
            .ok_or(DomainError::MissingIndexGeneration { id })?
            .transition_to(next)?;

        if next == IndexLifecycle::Active {
            self.active_generations.insert(name.clone(), id);
        } else if previous == IndexLifecycle::Active
            && let Some(&current_active) = self.active_generations.get(&name)
            && current_active == id
        {
            self.active_generations.remove(&name);
        }

        Ok(previous_active)
    }
}
