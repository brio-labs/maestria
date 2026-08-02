use maestria_domain::*;
#[path = "evidence.rs"]
mod evidence;
#[path = "file_evidence.rs"]
mod file_evidence;

pub fn state_with_memory_candidate(
    candidate_id: MemoryCandidateId,
) -> Result<KernelState, Box<dyn std::error::Error>> {
    let mut state = KernelState::new();
    evidence::register_artifact_and_claim(&mut state)?;
    let trusted_security = SecurityMetadata {
        trust_zone: TrustZone::Verified,
        authority: Authority::User,
        ..SecurityMetadata::default()
    };
    state
        .artifacts
        .get_mut(&ArtifactId::new(1))
        .ok_or(DomainError::MissingArtifact {
            id: ArtifactId::new(1),
        })?
        .security = trusted_security.clone();
    state
        .claims
        .get_mut(&ClaimId::new(20))
        .ok_or(DomainError::MissingClaim {
            id: ClaimId::new(20),
        })?
        .security = trusted_security.clone();
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_evidence::file_span_kind()?,
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(12),
        security: Some(trusted_security.clone()),
    }))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id,
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 720,
            security: Some(SecurityMetadata {
                trust_zone: TrustZone::Verified,
                authority: Authority::User,
                ..SecurityMetadata::default()
            }),
        },
    ))?;
    Ok(state)
}

pub fn promote_memory(
    state: &mut KernelState,
    memory_id: MemoryId,
    candidate_id: MemoryCandidateId,
) -> Result<(), DomainError> {
    state.apply_input(DomainInput::PromoteMemory(PromoteMemoryInput {
        memory_id,
        candidate_id,
    }))?;
    Ok(())
}
