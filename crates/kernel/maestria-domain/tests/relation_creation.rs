use maestria_domain::*;
#[path = "common/evidence.rs"]
mod common;
#[path = "common/file_evidence.rs"]
mod file_evidence;

use common::register_artifact_and_claim;
use file_evidence::file_span_kind;

// ── Relation and memory-candidate creation are evidence-bound ─────

fn assert_relation_created_with_valid_evidence(state: &mut KernelState) -> Result<(), DomainError> {
    let relation_output = state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: RelationId::new(70),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: Some(EvidenceId::new(40)),
        confidence_milli: 875,
        security: None,
    }))?;
    assert_eq!(
        state.relations.get(&RelationId::new(70)),
        Some(&Relation {
            id: RelationId::new(70),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(1)),
            evidence_id: Some(EvidenceId::new(40)),
            confidence_milli: 875,
            security: SecurityMetadata::default(),
        })
    );
    assert_eq!(
        relation_output.effects,
        vec![
            MaestriaEffect::PersistEvent {
                envelope: Box::new(relation_output.events[0].clone()),
            },
            MaestriaEffect::UpdateGraph(UpdateGraphRequest {
                relation_id: RelationId::new(70),
            }),
        ]
    );
    Ok(())
}
fn assert_relation_created_without_evidence_skips_graph_update(
    state: &mut KernelState,
) -> Result<(), DomainError> {
    let relation_output = state.apply_input(DomainInput::CreateRelation(CreateRelationInput {
        relation_id: RelationId::new(71),
        source: RelationEndpoint::Claim(ClaimId::new(20)),
        kind: RelationKind::Supports,
        target: RelationEndpoint::Artifact(ArtifactId::new(1)),
        evidence_id: None,
        confidence_milli: 875,
        security: None,
    }))?;
    assert_eq!(
        state.relations.get(&RelationId::new(71)),
        Some(&Relation {
            id: RelationId::new(71),
            source: RelationEndpoint::Claim(ClaimId::new(20)),
            kind: RelationKind::Supports,
            target: RelationEndpoint::Artifact(ArtifactId::new(1)),
            evidence_id: None,
            confidence_milli: 875,
            security: SecurityMetadata::default(),
        })
    );
    assert_eq!(
        relation_output.effects,
        vec![MaestriaEffect::PersistEvent {
            envelope: Box::new(relation_output.events[0].clone()),
        }]
    );
    Ok(())
}

fn assert_memory_candidate_created_with_evidence(
    state: &mut KernelState,
) -> Result<(), DomainError> {
    let candidate_output = state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(90),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40), EvidenceId::new(40)],
            confidence_milli: 720,
            security: None,
        },
    ))?;
    assert!(matches!(
        candidate_output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryCandidateCreated {
                candidate_id,
                claim_id,
                ..
            },
            ..
        }] if *candidate_id == MemoryCandidateId::new(90)
            && *claim_id == ClaimId::new(20)
    ));
    let candidate = state
        .memory_candidates
        .get(&MemoryCandidateId::new(90))
        .ok_or(DomainError::MissingMemoryCandidate {
            id: MemoryCandidateId::new(90),
        })?;
    assert!(candidate.has_evidence());
    assert_eq!(candidate.claim_id, ClaimId::new(20));
    assert_eq!(
        candidate.evidence_ids,
        std::collections::BTreeSet::from([EvidenceId::new(40)])
    );
    assert_eq!(candidate.confidence_milli, 720);
    Ok(())
}

#[test]
fn relation_and_memory_candidates_are_domain_owned_and_evidence_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        security: None,
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_span_kind()?,
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(12),
    }))?;

    assert_eq!(
        state
            .apply_input(DomainInput::CreateRelation(CreateRelationInput {
                relation_id: RelationId::new(99),
                security: None,
                source: RelationEndpoint::Claim(ClaimId::new(20)),
                kind: RelationKind::Supports,
                target: RelationEndpoint::Artifact(ArtifactId::new(1)),
                evidence_id: Some(EvidenceId::new(404)),
                confidence_milli: 875,
            }))
            .err(),
        Some(DomainError::MissingEvidence {
            id: EvidenceId::new(404)
        })
    );

    assert_relation_created_without_evidence_skips_graph_update(&mut state)?;
    assert_relation_created_with_valid_evidence(&mut state)?;

    assert!(matches!(
        state.apply_input(DomainInput::CreateMemoryCandidate(
            CreateMemoryCandidateInput {
                candidate_id: MemoryCandidateId::new(91),
                claim_id: ClaimId::new(20),
                evidence_ids: Vec::new(),
                confidence_milli: 720,
                security: None,
            },
        )),
        Err(DomainError::EvidenceRequired {
            kind: "memory_candidate",
            id: 91,
        })
    ));

    assert_memory_candidate_created_with_evidence(&mut state)?;
    Ok(())
}
