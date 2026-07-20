pub trait GraphIndex: Send + Sync {
    fn insert_relation(&self, relation: maestria_domain::Relation) -> Result<(), crate::PortError>;
    fn get_relations_for(
        &self,
        endpoint: maestria_domain::RelationEndpoint,
    ) -> Result<Vec<maestria_domain::Relation>, crate::PortError>;
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
