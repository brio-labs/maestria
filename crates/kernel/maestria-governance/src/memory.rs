use maestria_domain::MemoryCandidate;

/// Request to evaluate a memory candidate for promotion.
#[derive(Debug)]
pub struct MemoryPromotionRequest {
    pub candidate: MemoryCandidate,
    pub user_approved: bool,
}

/// Outcome of a memory promotion gate evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryPromotionDecision {
    Promote,
    RequireEvidence { reason: String },
    RequireReview { reason: String },
    Deny { reason: String },
}

/// Gate that decides whether a memory candidate should be promoted.
pub trait MemoryPromotionGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision;
}

/// Default memory promotion gate.
#[derive(Debug)]
pub struct DefaultMemoryPromotionGate;

impl MemoryPromotionGate for DefaultMemoryPromotionGate {
    fn evaluate(&self, request: &MemoryPromotionRequest) -> MemoryPromotionDecision {
        if !request.candidate.has_evidence() {
            return MemoryPromotionDecision::RequireEvidence {
                reason: "memory candidate must contain at least one evidence id".to_string(),
            };
        }
        if !request.candidate.security.memory_promotion_allowed() {
            return MemoryPromotionDecision::Deny {
                reason: "memory candidate security metadata blocks promotion".to_string(),
            };
        }

        if request.candidate.confidence_milli < 500 {
            return MemoryPromotionDecision::RequireReview {
                reason: "low confidence memory candidate".to_string(),
            };
        }

        if request.user_approved {
            MemoryPromotionDecision::Promote
        } else {
            MemoryPromotionDecision::RequireReview {
                reason: "user approval required for promotion".to_string(),
            }
        }
    }
}
