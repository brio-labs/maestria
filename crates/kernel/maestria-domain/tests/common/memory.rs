use maestria_domain::*;
#[path = "content_hash.rs"]
mod fixtures;

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

pub fn file_span_kind() -> Result<EvidenceKind, Box<dyn std::error::Error>> {
    Ok(EvidenceKind::FileSpan {
        path: "notes.txt".to_string(),
        range: LineRange::new(2, 2)?,
        snapshot: SnapshotRef::new(BlobId::new(42), fixtures::test_content_hash()?),
    })
}
