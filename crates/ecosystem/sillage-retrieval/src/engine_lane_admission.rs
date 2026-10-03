use super::*;

fn lane_uses_primary_generation(descriptor: &crate::types::RetrieverDescriptor) -> bool {
    !descriptor.modality.eq_ignore_ascii_case("dense")
        && !descriptor.modality.eq_ignore_ascii_case("image")
        && !descriptor.modality.eq_ignore_ascii_case("sparse")
        && !descriptor.modality.eq_ignore_ascii_case("sparse-shadow")
}

pub(super) fn lane_generation_is_current(
    descriptor: &crate::types::RetrieverDescriptor,
    plan: &SearchPlan,
) -> bool {
    !lane_uses_primary_generation(descriptor) || descriptor.generation == plan.index_generation()
}

pub(super) fn lane_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    plan: &SearchPlan,
    web_requests_used: u32,
) -> bool {
    lane_generation_is_current(descriptor, plan)
        && !(descriptor.modality.eq_ignore_ascii_case("web")
            && web_requests_used >= plan.budgets().max_web_requests())
}

pub(super) fn serial_dispatch_required(
    retrievers: &[Arc<dyn CandidateRetriever>],
    plan: &SearchPlan,
    execution_usage: SearchExecutionUsage,
    web_requests_used: u32,
) -> bool {
    let lane_count = retrievers
        .iter()
        .filter(|retriever| lane_is_eligible(retriever.descriptor(), plan, web_requests_used))
        .count();
    lane_count > 0
        && (0..lane_count)
            .any(|lane| lane_budget(plan, execution_usage, lane_count, lane).is_none())
}
