use maestria_domain::*;

pub fn register_artifact_and_claim(state: &mut KernelState) -> Result<(), DomainError> {
    state.apply_input(DomainInput::RegisterArtifact(RegisterArtifactInput {
        artifact_id: ArtifactId::new(1),
        title: "Project Notes".to_string(),
        security: None,
    }))?;
    state.apply_input(DomainInput::CreateClaim(CreateClaimInput {
        claim_id: ClaimId::new(20),
        artifact_id: ArtifactId::new(1),
        text: "Claim from evidence".to_string(),
        evidence_ids: Vec::new(),
        security: None,
    }))?;
    Ok(())
}
