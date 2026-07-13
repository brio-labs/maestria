use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{GraphIndex, PortError, Relation, RelationEndpoint, RelationId};

#[derive(Clone, Default)]
pub struct InMemoryGraphIndex {
    relations: Arc<Mutex<BTreeMap<RelationId, Relation>>>,
}

impl InMemoryGraphIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

impl GraphIndex for InMemoryGraphIndex {
    fn insert_relation(&self, relation: Relation) -> Result<(), PortError> {
        let mut guard = self.relations.lock().map_err(|_| PortError::Internal {
            message: "graph index lock poisoned".to_string(),
        })?;
        guard.insert(relation.id, relation);
        Ok(())
    }

    fn get_relations_for(&self, endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError> {
        let guard = self.relations.lock().map_err(|_| PortError::Internal {
            message: "graph index lock poisoned".to_string(),
        })?;
        Ok(guard
            .values()
            .filter(|r| r.source == endpoint || r.target == endpoint)
            .cloned()
            .collect())
    }
}
