use maestria_domain::*;

pub fn register_artifact_and_claim(state: &mut KernelState) -> Result<(), DomainError> {
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
    }))?;
    state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id: ClaimId::new(20),
        artifact_id: ArtifactId::new(1),
        text: "Claim from evidence".to_string(),
        evidence_ids: Vec::new(),
    }))?;
    Ok(())
}

pub fn file_span_kind() -> EvidenceKind {
    EvidenceKind::FileSpan {
        path: "notes.txt".to_string(),
        range: ContentRange { start: 1, end: 2 },
        content_hash: "sha256:notes".to_string(),
        snapshot: None,
    }
}

pub fn state_with_memory_candidate(
    candidate_id: MemoryCandidateId,
) -> Result<KernelState, DomainError> {
    let mut state = KernelState::new();
    register_artifact_and_claim(&mut state)?;
    state.apply_input(DomainInput::RecordEvidence(RecordEvidenceInput {
        evidence_id: EvidenceId::new(40),
        artifact_id: ArtifactId::new(1),
        claim_id: Some(ClaimId::new(20)),
        kind: file_span_kind(),
        excerpt: "first chunk".to_string(),
        observed_at: LogicalTick::new(12),
    }))?;
    state.apply_input(DomainInput::CreateMemoryCandidate(
        CreateMemoryCandidateInput {
            candidate_id,
            claim_id: ClaimId::new(20),
            evidence_ids: vec![EvidenceId::new(40)],
            confidence_milli: 720,
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
