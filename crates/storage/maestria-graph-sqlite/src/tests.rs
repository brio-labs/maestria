use super::*;
use maestria_domain::{
    ArtifactId, CardId, ClaimId, EvidenceId, MemoryId, Relation, RelationEndpoint, RelationId,
    RelationKind, TaskId,
};
use maestria_ports::PortError;
use rusqlite::Connection;

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
        index.get_relations_for(RelationEndpoint::Claim(ClaimId::new(20)))?,
        vec![relation.clone()]
    );
    assert_eq!(
        index.get_relations_for(RelationEndpoint::Artifact(ArtifactId::new(1)))?,
        vec![relation]
    );
    assert!(
        index
            .get_relations_for(RelationEndpoint::Task(TaskId::new(50)))?
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
        index.get_relations_for(RelationEndpoint::Memory(MemoryId::new(8)))?,
        vec![updated]
    );
    assert!(
        index
            .get_relations_for(RelationEndpoint::Claim(ClaimId::new(20)))?
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

    let result = index.get_relations_for(RelationEndpoint::Artifact(ArtifactId::new(1)));
    assert!(matches!(
        result,
        Err(PortError::Internal { ref message }) if message.contains("unknown relation kind unknown_kind")
    ));
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

    let results = index.get_relations_for(RelationEndpoint::Claim(ClaimId::new(1)))?;

    // 2 should come before 10, whereas lexical sorting would put 10 before 2
    assert_eq!(results, vec![relation_2, relation_10]);
    Ok(())
}

#[test]
fn satisfies_graph_index_contract() -> Result<(), PortError> {
    let index = SqliteGraphIndex::in_memory()?;
    maestria_ports::graph_contract_tests::assert_graph_index_contract(&index);
    Ok(())
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
    assert_eq!(index.get_relations_for(ep)?.len(), 1);

    index.clear()?;
    assert!(index.get_relations_for(ep)?.is_empty());
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
    assert_eq!(index.get_relations_for(ep)?.len(), 1);
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
    assert_eq!(index.get_relations_for(ep)?.len(), 1);

    index.rebuild(vec![rel2.clone()])?;

    let current = index.get_relations_for(ep)?;
    assert_eq!(current.len(), 1);
    assert_eq!(current[0], rel2);
    Ok(())
}

#[test]
fn migration_is_idempotent() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;
    migrate(&mut conn)?; // Initial migration
    migrate(&mut conn)?; // Second should succeed without error (idempotent)
    Ok(())
}

#[test]
fn rejects_unsupported_schema_version() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;

    // Force an unsupported version manually
    conn.execute_batch(
        "CREATE TABLE graph_projection_schema (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             version INTEGER NOT NULL
         );
         INSERT INTO graph_projection_schema (id, version) VALUES (1, 9999);",
    )
    .map_err(to_port_error)?;

    let result = migrate(&mut conn);
    assert!(matches!(
        result,
        Err(PortError::Internal { ref message }) if message.contains("unsupported graph projection schema version 9999")
    ));
    Ok(())
}

#[test]
fn rejects_version_claiming_missing_schema() -> Result<(), PortError> {
    let mut conn = Connection::open_in_memory().map_err(to_port_error)?;

    conn.execute_batch(
        "CREATE TABLE graph_projection_schema (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             version INTEGER NOT NULL
         );
         INSERT INTO graph_projection_schema (id, version) VALUES (1, 1);",
    )
    .map_err(to_port_error)?;

    let result = migrate(&mut conn);
    assert!(matches!(
        result,
        Err(PortError::Internal { ref message }) if message.contains("relations table is malformed or missing columns")
    ));
    Ok(())
}
