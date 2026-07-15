use super::test_support::*;
use maestria_domain::{
    ArtifactId, ClaimId, MaestriaEffect, Relation, RelationEndpoint, RelationId, RelationKind,
    UpdateGraphRequest,
};
use maestria_ports::{GraphIndex, PortError};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

struct FailingGraphIndex;
impl GraphIndex for FailingGraphIndex {
    fn insert_relation(&self, _relation: Relation) -> Result<(), PortError> {
        Err(PortError::Internal {
            message: "forced failure".into(),
        })
    }
    fn get_relations_for(&self, _endpoint: RelationEndpoint) -> Result<Vec<Relation>, PortError> {
        Ok(vec![])
    }
    fn delete_relations(&self, _relation_ids: &[RelationId]) -> Result<(), PortError> {
        Ok(())
    }
    fn clear(&self) -> Result<(), PortError> {
        Ok(())
    }
}

#[tokio::test]
async fn update_graph_inserts_relation_when_present() {
    let relation_id = RelationId::new(1);
    let relation = Relation {
        id: relation_id,
        source: RelationEndpoint::Claim(ClaimId::new(1)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(2)),
        evidence_id: Some(maestria_domain::EvidenceId::new(7)),
        confidence_milli: 800,
        security: maestria_domain::SecurityMetadata::default(),
    };

    let mut state = KernelState::new();
    state.relations.insert(relation_id, relation.clone());

    let adapters = crate::test_helpers::test_adapters();
    let graph_index = adapters.graph_index.clone();
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(state)),
        input_tx,
    );

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::UpdateGraph(UpdateGraphRequest { relation_id }),
        ctx,
        None,
    )
    .await;

    assert!(result, "update_graph should succeed");

    let stored = graph_index
        .get_relations_for(RelationEndpoint::Claim(ClaimId::new(1)))
        .expect("graph relation lookup");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0], relation);
}

#[tokio::test]
async fn update_graph_fails_when_relation_missing_from_state() {
    let adapters = crate::test_helpers::test_adapters();
    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(KernelState::new())),
        input_tx,
    );

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::UpdateGraph(UpdateGraphRequest {
            relation_id: RelationId::new(99),
        }),
        ctx,
        None,
    )
    .await;

    assert!(
        !result,
        "update_graph must fail if relation is not in state"
    );
}

#[tokio::test]
async fn update_graph_fails_when_adapter_fails() {
    let relation_id = RelationId::new(1);
    let mut state = KernelState::new();
    state.relations.insert(
        relation_id,
        Relation {
            id: relation_id,
            source: RelationEndpoint::Claim(ClaimId::new(1)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(2)),
            evidence_id: Some(maestria_domain::EvidenceId::new(7)),
            confidence_milli: 800,
            security: maestria_domain::SecurityMetadata::default(),
        },
    );

    let mut adapters = crate::test_helpers::test_adapters();
    adapters.graph_index = Arc::new(FailingGraphIndex);

    let governance = crate::test_helpers::test_governance();
    let (input_tx, _input_rx) = mpsc::channel(8);

    let ctx = EffectExecutionContext::test_default(
        Arc::new(adapters),
        Arc::new(governance),
        Arc::new(RwLock::new(state)),
        input_tx,
    );

    let result = MaestriaRuntime::test_execute_effect(
        MaestriaEffect::UpdateGraph(UpdateGraphRequest { relation_id }),
        ctx,
        None,
    )
    .await;

    assert!(!result, "update_graph must fail if adapter returns error");
}
