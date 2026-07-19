use super::batch_is_eligible;
use crate::types::{HybridExecutionPolicy, HybridPromotionRecord, RetrieverDescriptor};
use maestria_domain::{IndexGenerationId, RepresentationName};

fn descriptor(id: &str, modality: &str) -> RetrieverDescriptor {
    RetrieverDescriptor {
        id: id.to_string(),
        modality: modality.to_string(),
        representation: RepresentationName::new("test"),
        generation: IndexGenerationId::new(1),
    }
}

fn active_hybrid_policy() -> Result<HybridExecutionPolicy, &'static str> {
    let Some(record) = HybridPromotionRecord::new("hybrid".to_string(), "2026-07-18".to_string())
    else {
        return Err("valid test promotion record was rejected");
    };
    Ok(HybridExecutionPolicy::Active(record))
}

#[test]
fn repository_code_lane_is_shadowed_until_promoted_for_query_class() -> Result<(), &'static str> {
    let code = descriptor("code_intel_symbols", "code");
    let hybrid = active_hybrid_policy()?;
    assert!(!batch_is_eligible(&code, &hybrid, false));
    assert!(batch_is_eligible(&code, &hybrid, true));
    Ok(())
}

#[test]
fn dense_shadow_filter_remains_independent_of_repository_policy() -> Result<(), &'static str> {
    let dense = descriptor("dense", "text");
    assert!(!batch_is_eligible(
        &dense,
        &HybridExecutionPolicy::Shadow,
        true
    ));
    assert!(batch_is_eligible(&dense, &active_hybrid_policy()?, true));
    Ok(())
}
