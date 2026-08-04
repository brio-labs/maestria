use super::*;
use maestria_domain::{EvidenceId, MemoryCandidateId};

fn candidate_with_artifact(id: u64, has_evidence: bool) -> maestria_domain::MemoryCandidate {
    let mut evidence_ids = std::collections::BTreeSet::new();
    if has_evidence {
        evidence_ids.insert(EvidenceId::new(id));
    }

    maestria_domain::MemoryCandidate {
        id: MemoryCandidateId::new(id),
        claim_id: maestria_domain::ClaimId::new(id),
        evidence_ids,
        confidence_milli: 900,
        security: maestria_domain::SecurityMetadata::default(),
    }
}

#[test]
fn memory_promotion_gate_requires_evidence() {
    let candidate = candidate_with_artifact(42, false);
    let request = MemoryPromotionRequest {
        candidate,
        user_approved: true,
    };

    let decision = DefaultMemoryPromotionGate.evaluate(&request);
    assert!(matches!(
        decision,
        MemoryPromotionDecision::RequireEvidence { .. }
    ));
}

#[test]
fn memory_promotion_denies_tainted_candidate() {
    let mut candidate = candidate_with_artifact(43, true);
    candidate.security.prompt_injection_risk = true;
    let decision = DefaultMemoryPromotionGate.evaluate(&MemoryPromotionRequest {
        candidate,
        user_approved: true,
    });
    assert!(matches!(decision, MemoryPromotionDecision::Deny { .. }));
}
