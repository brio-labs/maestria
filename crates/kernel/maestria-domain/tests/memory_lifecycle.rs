use maestria_domain::*;
use std::collections::BTreeSet;
#[path = "common/memory_lifecycle.rs"]
mod common;

use common::{promote_memory, state_with_memory_candidate};

// ── Memory lifecycle transitions are evented and status-owned ─────

#[test]
fn promote_memory_creates_active_memory_from_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;

    let output = state.apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
        memory_id: MemoryId::new(100),
        candidate_id: MemoryCandidateId::new(90),
    }))?;

    let memory = state
        .memories
        .get(&MemoryId::new(100))
        .ok_or(DomainError::MissingMemory {
            id: MemoryId::new(100),
        })?;
    assert_eq!(memory.candidate_id, MemoryCandidateId::new(90));
    assert_eq!(memory.claim_id, ClaimId::new(20));
    assert_eq!(memory.evidence_ids, BTreeSet::from([EvidenceId::new(40)]));
    assert_eq!(memory.status, MemoryStatus::Active);
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryPromoted {
                memory_id,
                candidate_id,
                ..
            },
            ..
        }] if *memory_id == MemoryId::new(100)
            && *candidate_id == MemoryCandidateId::new(90)
    ));
    assert_eq!(
        output.effects,
        vec![MaestriaEffect::PersistEvent {
            envelope: Box::new(output.events[0].clone()),
        }]
    );
    Ok(())
}

#[test]
fn promote_memory_rejects_missing_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = KernelState::new();

    assert_eq!(
        state
            .apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
                memory_id: MemoryId::new(100),
                candidate_id: MemoryCandidateId::new(404),
            }))
            .err(),
        Some(DomainError::MissingMemoryCandidate {
            id: MemoryCandidateId::new(404),
        })
    );
    Ok(())
}

#[test]
fn contradict_memory_marks_memory_contradicted() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(91),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 650,
            security: None,
        },
    ))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

    let output = state.apply_input(DomainInput::ContradictMemory(ContradictMemoryInput {
        memory_id: MemoryId::new(100),
        contradicting_candidate_id: MemoryCandidateId::new(91),
    }))?;

    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Contradicted
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryContradicted {
                memory_id,
                contradicting_candidate_id,
            },
            ..
        }] if *memory_id == MemoryId::new(100)
            && *contradicting_candidate_id == MemoryCandidateId::new(91)
    ));
    Ok(())
}

#[test]
fn deprecate_memory_marks_memory_deprecated() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

    let output = state.apply_input(DomainInput::DeprecateMemory(DeprecateMemoryInput {
        memory_id: MemoryId::new(100),
    }))?;

    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Deprecated
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemoryDeprecated { memory_id },
            ..
        }] if *memory_id == MemoryId::new(100)
    ));
    Ok(())
}

#[test]
fn supersede_memory_marks_memory_superseded() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id: MemoryCandidateId::new(91),
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 650,
            security: Some(SecurityMetadata {
                trust_zone: TrustZone::Verified,
                authority: Authority::User,
                ..SecurityMetadata::default()
            }),
        },
    ))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;
    promote_memory(&mut state, MemoryId::new(101), MemoryCandidateId::new(91))?;

    let output = state.apply_input(DomainInput::SupersedeMemory(SupersedeMemoryInput {
        memory_id: MemoryId::new(100),
        by_memory_id: MemoryId::new(101),
    }))?;

    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Superseded
    );
    assert!(matches!(
        output.events.as_slice(),
        [DomainEventEnvelope {
            event: DomainEvent::MemorySuperseded {
                memory_id,
                by_memory_id,
            },
            ..
        }] if *memory_id == MemoryId::new(100)
            && *by_memory_id == MemoryId::new(101)
    ));
    Ok(())
}

#[test]
fn supersede_memory_rejects_self_reference() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state_with_memory_candidate(MemoryCandidateId::new(90))?;
    promote_memory(&mut state, MemoryId::new(100), MemoryCandidateId::new(90))?;

    let result = state.apply_input(DomainInput::SupersedeMemory(SupersedeMemoryInput {
        memory_id: MemoryId::new(100),
        by_memory_id: MemoryId::new(100),
    }));

    assert!(matches!(
        result,
        Err(DomainError::MemorySupersedesItself { memory_id })
            if memory_id == MemoryId::new(100)
    ));
    assert_eq!(
        state
            .memories
            .get(&MemoryId::new(100))
            .ok_or(DomainError::MissingMemory {
                id: MemoryId::new(100),
            })?
            .status,
        MemoryStatus::Active
    );
    Ok(())
}
