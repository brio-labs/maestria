use super::super::contract_tests::*;
use super::super::graph_contract_tests::assert_graph_index_contract;
use super::super::*;
use maestria_domain::RelationEndpoint;

fn graph_query(endpoint: RelationEndpoint) -> Result<GraphRelationQuery, PortError> {
    GraphRelationQuery::new(endpoint, u64::MAX).ok_or_else(|| PortError::Internal {
        message: "graph query limit must be positive".to_string(),
    })
}

#[test]
fn in_memory_full_text_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_full_text_index_round_trip(&InMemoryFullTextIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_vector_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_vector_index_contract(&InMemoryVectorIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_graph_index_satisfies_contract() -> Result<(), Box<dyn std::error::Error>> {
    assert_graph_index_contract(&InMemoryGraphIndex::new())?;
    Ok(())
}

#[test]
fn in_memory_graph_index_clear_removes_all_relations() -> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryGraphIndex::new();
    let ep = RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(1));
    let rel = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(maestria_domain::CardId::new(2)),
        kind: maestria_domain::RelationKind::Contains,
        evidence_id: Some(maestria_domain::EvidenceId::new(3)),
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
fn in_memory_graph_index_delete_relations_ignores_empty_list()
-> Result<(), Box<dyn std::error::Error>> {
    let index = InMemoryGraphIndex::new();
    let ep = RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(1));
    let rel = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(maestria_domain::CardId::new(2)),
        kind: maestria_domain::RelationKind::Contains,
        evidence_id: Some(maestria_domain::EvidenceId::new(3)),
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
fn in_memory_graph_index_rebuild_preserves_new_relations() -> Result<(), Box<dyn std::error::Error>>
{
    let index = InMemoryGraphIndex::new();
    let ep = RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(1));
    let rel1 = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(1),
        source: ep,
        target: RelationEndpoint::Card(maestria_domain::CardId::new(2)),
        kind: maestria_domain::RelationKind::Contains,
        evidence_id: Some(maestria_domain::EvidenceId::new(3)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel2 = maestria_domain::Relation {
        id: maestria_domain::RelationId::new(2),
        source: ep,
        target: RelationEndpoint::Claim(maestria_domain::ClaimId::new(4)),
        kind: maestria_domain::RelationKind::Supports,
        evidence_id: Some(maestria_domain::EvidenceId::new(5)),
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
