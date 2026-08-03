use crate::conversion::to_port_error;
use crate::graph::SqliteGraphIndex;

use maestria_domain::{
    ArtifactId, CardId, ClaimId, EvidenceId, MemoryId, Relation, RelationEndpoint, RelationId,
    RelationKind,
};
use maestria_ports::{GraphIndex, GraphRelationQuery, PortError};

fn graph_query(endpoint: RelationEndpoint) -> Result<GraphRelationQuery, PortError> {
    GraphRelationQuery::new(endpoint, u64::MAX).ok_or_else(|| PortError::Internal {
        message: "graph query limit must be positive".to_string(),
    })
}

#[test]
fn clear_removes_all_relations() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let ep = RelationEndpoint::Artifact(ArtifactId::new(1));
    let rel = Relation {
        id: RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(CardId::new(2)),
        kind: RelationKind::Contains,
        evidence_id: Some(EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    index.insert_relation(rel.clone())?;
    assert_eq!(
        index.get_relations_for(graph_query(ep)?)?.relations.len(),
        1
    );

    index.clear()?;
    assert!(
        index
            .get_relations_for(graph_query(ep)?)?
            .relations
            .is_empty()
    );
    Ok(())
}

#[test]
fn delete_relations_ignores_empty_list() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let ep = RelationEndpoint::Artifact(ArtifactId::new(1));
    let rel = Relation {
        id: RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(CardId::new(2)),
        kind: RelationKind::Contains,
        evidence_id: Some(EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    index.insert_relation(rel.clone())?;

    index.delete_relations(&[])?;
    assert_eq!(
        index.get_relations_for(graph_query(ep)?)?.relations.len(),
        1
    );
    Ok(())
}

#[test]
fn rebuild_preserves_new_relations() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let ep = RelationEndpoint::Artifact(ArtifactId::new(1));
    let rel1 = Relation {
        id: RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(CardId::new(2)),
        kind: RelationKind::Contains,
        evidence_id: Some(EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel2 = Relation {
        id: RelationId::new(2),
        source: ep,
        target: RelationEndpoint::Claim(ClaimId::new(4)),
        kind: RelationKind::Supports,
        evidence_id: Some(EvidenceId::new(5)),
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    };

    index.insert_relation(rel1.clone())?;
    assert_eq!(
        index.get_relations_for(graph_query(ep)?)?.relations.len(),
        1
    );

    index.rebuild(vec![rel2.clone()])?;

    let current = index.get_relations_for(graph_query(ep)?)?;
    assert_eq!(current.relations.len(), 1);
    assert_eq!(current.relations[0], rel2);
    Ok(())
}

#[test]
fn rebuild_rolls_back_when_insert_fails() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let ep = RelationEndpoint::Artifact(ArtifactId::new(1));
    let rel1 = Relation {
        id: RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(CardId::new(2)),
        kind: RelationKind::Contains,
        evidence_id: Some(EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel2 = Relation {
        id: RelationId::new(2),
        source: ep,
        target: RelationEndpoint::Claim(ClaimId::new(4)),
        kind: RelationKind::Supports,
        evidence_id: Some(EvidenceId::new(5)),
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel3 = Relation {
        id: RelationId::new(3),
        source: ep,
        target: RelationEndpoint::Memory(MemoryId::new(6)),
        kind: RelationKind::RelatedTo,
        evidence_id: None,
        confidence_milli: 700,
        security: maestria_domain::SecurityMetadata::default(),
    };

    index.insert_relation(rel1.clone())?;
    let connection = index.lock_connection()?;
    connection
        .execute_batch(
            "CREATE TRIGGER reject_rebuild_relation
             BEFORE INSERT ON relations
             WHEN NEW.id = '3'
             BEGIN
                 SELECT RAISE(ABORT, 'injected rebuild failure');
             END;",
        )
        .map_err(to_port_error)?;
    drop(connection);

    let result = index.rebuild(vec![rel2, rel3]);
    assert!(result.is_err_and(|error| error.is_internal()));
    assert_eq!(
        index.get_relations_for(graph_query(ep)?)?.relations,
        vec![rel1]
    );
    assert!(
        index
            .get_relations_for(graph_query(RelationEndpoint::Memory(MemoryId::new(6)))?)?
            .relations
            .is_empty()
    );
    Ok(())
}
