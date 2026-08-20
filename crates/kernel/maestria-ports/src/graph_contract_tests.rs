use super::*;
use maestria_domain::{
    ArtifactId, CardId, ClaimId, EvidenceId, Relation, RelationEndpoint, RelationId, RelationKind,
};
pub fn graph_query(endpoint: RelationEndpoint) -> Result<GraphRelationQuery, crate::PortError> {
    GraphRelationQuery::new(endpoint, u64::MAX).ok_or_else(|| {
        crate::PortError::internal("graph query", "graph query limit must be positive")
    })
}

pub fn assert_graph_index_contract(
    index: &impl GraphIndex,
) -> Result<(), Box<dyn std::error::Error>> {
    let artifact_ep = RelationEndpoint::Artifact(ArtifactId::new(1));
    let card_ep = RelationEndpoint::Card(CardId::new(2));
    let claim_ep = RelationEndpoint::Claim(ClaimId::new(3));

    let mut rel3 = Relation {
        id: RelationId::new(3),
        source: artifact_ep,
        target: card_ep,
        kind: RelationKind::Contains,
        evidence_id: Some(EvidenceId::new(5)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel1 = Relation {
        id: RelationId::new(1),
        source: card_ep,
        target: claim_ep,
        kind: RelationKind::Supports,
        evidence_id: Some(EvidenceId::new(4)),
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    };
    let rel2 = Relation {
        id: RelationId::new(2),
        source: artifact_ep,
        target: claim_ep,
        kind: RelationKind::Contradicts,
        evidence_id: Some(EvidenceId::new(6)),
        confidence_milli: 500,
        security: maestria_domain::SecurityMetadata::default(),
    };

    index.insert_relation(rel3.clone())?;
    index.insert_relation(rel1.clone())?;
    index.insert_relation(rel2.clone())?;

    rel3.confidence_milli = 950;
    index.insert_relation(rel3.clone())?;

    let artifact_rels = index.get_relations_for(graph_query(artifact_ep)?)?;
    assert_eq!(artifact_rels.relations, vec![rel2.clone(), rel3.clone()]);
    assert!(artifact_rels.complete);

    let claim_rels = index.get_relations_for(graph_query(claim_ep)?)?;
    assert_eq!(claim_rels.relations, vec![rel1.clone(), rel2.clone()]);
    assert!(claim_rels.complete);
    let bounded_claim_rels = index.get_relations_for(
        GraphRelationQuery::new(claim_ep, 1).ok_or("graph query limit must be positive")?,
    )?;
    assert_eq!(bounded_claim_rels.relations, vec![rel1.clone()]);
    assert!(!bounded_claim_rels.complete);

    index.clear()?;
    assert!(
        index
            .get_relations_for(graph_query(artifact_ep)?)?
            .relations
            .is_empty()
    );
    assert!(
        index
            .get_relations_for(graph_query(card_ep)?)?
            .relations
            .is_empty()
    );
    assert!(
        index
            .get_relations_for(graph_query(claim_ep)?)?
            .relations
            .is_empty()
    );

    index.rebuild(vec![rel1.clone(), rel2.clone(), rel3.clone()])?;
    let rebuilt_rels = index.get_relations_for(graph_query(claim_ep)?)?;
    assert_eq!(rebuilt_rels.relations, vec![rel1.clone(), rel2]);
    assert!(rebuilt_rels.complete);

    index.delete_relations(&[])?;
    assert_eq!(
        index
            .get_relations_for(graph_query(claim_ep)?)?
            .relations
            .len(),
        2
    );
    index.delete_relations(&[RelationId::new(2)])?;
    let after_delete = index.get_relations_for(graph_query(claim_ep)?)?;
    assert_eq!(after_delete.relations, vec![rel1]);
    Ok(())
}
