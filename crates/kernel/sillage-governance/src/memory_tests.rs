use super::*;
use sillage_domain::{EvidenceId, MemoryCandidateId};

#[test]
fn memory_promotion_denies_tainted_candidate() -> Result<(), Box<dyn std::error::Error>> {
    let security = sillage_domain::SecurityMetadata {
        prompt_injection_risk: true,
        ..sillage_domain::SecurityMetadata::default()
    };
    let candidate = sillage_domain::MemoryCandidate::try_new(
        MemoryCandidateId::new(43),
        sillage_domain::ClaimId::new(43),
        std::collections::BTreeSet::from([EvidenceId::new(43)]),
        900,
        security,
    )?;
    let decision = DefaultMemoryPromotionGate.evaluate(&MemoryPromotionRequest {
        candidate,
        user_approved: true,
    });
    assert!(matches!(decision, MemoryPromotionDecision::Deny { .. }));
    Ok(())
}
