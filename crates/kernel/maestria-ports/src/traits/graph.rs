use std::num::NonZeroU64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRelationQuery {
    endpoint: maestria_domain::RelationEndpoint,
    max_relations: NonZeroU64,
}

impl GraphRelationQuery {
    pub fn new(endpoint: maestria_domain::RelationEndpoint, max_relations: u64) -> Option<Self> {
        NonZeroU64::new(max_relations).map(|max_relations| Self {
            endpoint,
            max_relations,
        })
    }

    pub fn endpoint(&self) -> maestria_domain::RelationEndpoint {
        self.endpoint
    }

    pub fn max_relations(&self) -> u64 {
        self.max_relations.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRelationPage {
    pub relations: Vec<maestria_domain::Relation>,
    pub complete: bool,
}

pub trait GraphIndex: Send + Sync {
    fn insert_relation(&self, relation: maestria_domain::Relation) -> Result<(), crate::PortError>;
    fn get_relations_for(
        &self,
        query: GraphRelationQuery,
    ) -> Result<GraphRelationPage, crate::PortError>;
    fn delete_relations(
        &self,
        relation_ids: &[maestria_domain::RelationId],
    ) -> Result<(), crate::PortError>;
    fn clear(&self) -> Result<(), crate::PortError>;
    fn rebuild(&self, relations: Vec<maestria_domain::Relation>) -> Result<(), crate::PortError> {
        self.clear()?;
        for relation in relations {
            self.insert_relation(relation)?;
        }
        Ok(())
    }
}
