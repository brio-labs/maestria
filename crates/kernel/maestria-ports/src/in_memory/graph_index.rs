use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::store::lock_map;
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
        let mut guard = lock_map(&self.relations, "graph index lock poisoned")?;
        guard.insert(relation.id, relation);
        Ok(())
    }

    fn get_relations_for(&self, query: GraphRelationQuery) -> Result<GraphRelationPage, PortError> {
        let guard = lock_map(&self.relations, "graph index lock poisoned")?;
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
        let mut guard = lock_map(&self.relations, "graph index lock poisoned")?;
        for id in relation_ids {
            guard.remove(id);
        }
        Ok(())
    }

    fn clear(&self) -> Result<(), PortError> {
        let mut guard = lock_map(&self.relations, "graph index lock poisoned")?;
        guard.clear();
        Ok(())
    }
}
