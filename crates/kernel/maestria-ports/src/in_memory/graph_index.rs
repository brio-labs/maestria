use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{GraphIndex, GraphRelationPage, GraphRelationQuery, PortError};
use maestria_domain::{Relation, RelationId};

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
        let mut guard = self
            .relations
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "graph index lock poisoned",
                source: "graph relation mutex is poisoned".to_string(),
            })?;
        guard.insert(relation.id, relation);
        Ok(())
    }

    fn get_relations_for(&self, query: GraphRelationQuery) -> Result<GraphRelationPage, PortError> {
        let guard = self
            .relations
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "graph index lock poisoned",
                source: "graph relation mutex is poisoned".to_string(),
            })?;
        let max_relations = maestria_domain::saturating_usize(query.max_relations());
        let mut relations = guard
            .values()
            .filter(|relation| {
                relation.source == query.endpoint() || relation.target == query.endpoint()
            })
            .take(max_relations.saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        let complete = relations.len() <= max_relations;
        if !complete {
            relations.truncate(max_relations);
        }
        Ok(GraphRelationPage {
            relations,
            complete,
        })
    }

    fn delete_relations(&self, relation_ids: &[RelationId]) -> Result<(), PortError> {
        let mut guard = self
            .relations
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "graph index lock poisoned",
                source: "graph relation mutex is poisoned".to_string(),
            })?;
        for id in relation_ids {
            guard.remove(id);
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = self
            .relations
            .lock()
            .map_err(|_| PortError::InternalContext {
                context: "graph index lock poisoned",
                source: "graph relation mutex is poisoned".to_string(),
            })?;
        guard.clear();
        Ok(())
    }
}
