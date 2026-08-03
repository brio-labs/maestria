use crate::conversion::to_port_error;
use crate::graph::SqliteGraphIndex;

use maestria_domain::{
    ArtifactId, CardId, ClaimId, EvidenceId, MemoryId, Relation, RelationEndpoint, RelationId,
    RelationKind, TaskId,
};
use maestria_ports::{GraphIndex, GraphRelationQuery, PortError};

fn graph_query(endpoint: RelationEndpoint) -> Result<GraphRelationQuery, PortError> {
    GraphRelationQuery::new(endpoint, u64::MAX).ok_or_else(|| PortError::Internal {
        message: "graph query limit must be positive".to_string(),
    })
}

#[test]
fn returns_relations_matching_either_endpoint() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let relation = Relation {
        id: RelationId::new(7),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: Some(EvidenceId::new(40)),
        confidence_milli: 875,
        security: maestria_domain::SecurityMetadata {
            prompt_injection_risk: true,
            ..maestria_domain::SecurityMetadata::default()
        },
    };

    index.insert_relation(relation.clone())?;

    assert_eq!(
        index
            .get_relations_for(graph_query(RelationEndpoint::Claim(ClaimId::new(20)))?)?
            .relations,
        vec![relation.clone()]
    );
    assert_eq!(
        index
            .get_relations_for(graph_query(RelationEndpoint::Artifact(ArtifactId::new(1)))?)?
            .relations,
        vec![relation]
    );
    assert!(
        index
            .get_relations_for(graph_query(RelationEndpoint::Task(TaskId::new(50)))?)?
            .relations
            .is_empty()
    );
    Ok(())
}

#[test]
fn insert_relation_updates_existing_row() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let first = Relation {
        id: RelationId::new(7),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: Some(EvidenceId::new(40)),
        confidence_milli: 875,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let updated = Relation {
        id: RelationId::new(7),
        source: RelationEndpoint::Memory(MemoryId::new(8)),
        kind: RelationKind::RelatedTo,
        target: RelationEndpoint::Card(CardId::new(9)),
        evidence_id: None,
        confidence_milli: 600,
        security: maestria_domain::SecurityMetadata::default(),
    };

    index.insert_relation(first)?;
    index.insert_relation(updated.clone())?;

    assert_eq!(
        index
            .get_relations_for(graph_query(RelationEndpoint::Memory(MemoryId::new(8)))?)?
            .relations,
        vec![updated]
    );
    assert!(
        index
            .get_relations_for(graph_query(RelationEndpoint::Claim(ClaimId::new(20)))?)?
            .relations
            .is_empty()
    );
    Ok(())
}

#[test]
fn returns_error_for_invalid_stored_relation() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;

    let connection = index.lock_connection()?;
    connection.execute(
        "INSERT INTO relations (id, source_type, source_id, kind, target_type, target_id, confidence_milli)
         VALUES ('123', 'artifact', '1', 'unknown_kind', 'claim', '2', 1000)",
        [],
    ).map_err(to_port_error)?;
    drop(connection);

    let result =
        index.get_relations_for(graph_query(RelationEndpoint::Artifact(ArtifactId::new(1)))?);
    assert!(result.is_err_and(|error| error.is_internal()));
    Ok(())
}

#[test]
fn orders_relations_by_numeric_id_not_lexical() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    let relation_10 = Relation {
        id: RelationId::new(10),
        source: RelationEndpoint::Claim(ClaimId::new(1)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(2)),
        evidence_id: None,
        confidence_milli: 1000,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let relation_2 = Relation {
        id: RelationId::new(2),
        source: RelationEndpoint::Claim(ClaimId::new(1)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(3)),
        evidence_id: None,
        confidence_milli: 1000,
        security: maestria_domain::SecurityMetadata::default(),
    };

    // Insert in arbitrary order
    index.insert_relation(relation_10.clone())?;
    index.insert_relation(relation_2.clone())?;

    let results =
        index.get_relations_for(graph_query(RelationEndpoint::Claim(ClaimId::new(1)))?)?;

    // 2 should come before 10, whereas lexical sorting would put 10 before 2
    assert_eq!(results.relations, vec![relation_2, relation_10]);
    Ok(())
}

#[test]
fn satisfies_graph_index_contract() -> Result<(), Box<dyn std::error::Error>> {
    let index = SqliteGraphIndex::in_memory()?;
    maestria_ports::graph_contract_tests::assert_graph_index_contract(&index)?;
    Ok(())
}
