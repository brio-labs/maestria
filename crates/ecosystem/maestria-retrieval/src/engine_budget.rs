use maestria_domain::{
    SearchExecution, SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionUsage,
    SearchPlan,
};

pub(crate) fn execution_with_budget(
    budget: SearchExecutionBudget,
    completion: SearchExecutionCompletion,
) -> SearchExecution {
    SearchExecution::new(budget, SearchExecutionUsage::default(), completion)
}

pub(crate) fn add_usage(total: &mut SearchExecutionUsage, usage: SearchExecutionUsage) {
    total.results = total.results.saturating_add(usage.results);
    total.candidates = total.candidates.saturating_add(usage.candidates);
    total.work_units = total.work_units.saturating_add(usage.work_units);
    total.bytes_read = total.bytes_read.saturating_add(usage.bytes_read);
}

pub(crate) fn usage_within_budget(
    usage: SearchExecutionUsage,
    budget: SearchExecutionBudget,
) -> bool {
    usage.results <= budget.max_results()
        && usage.candidates <= budget.max_candidates()
        && usage.work_units <= budget.max_work_units()
        && budget
            .max_bytes_read()
            .is_none_or(|limit| usage.bytes_read <= limit.get())
}

fn partition_allowance(total: u64, lanes: usize, lane: usize) -> u64 {
    let lanes = lanes.max(1) as u64;
    let base = total / lanes;
    let remainder = total % lanes;
    base + if (lane as u64) < remainder { 1 } else { 0 }
}

pub(crate) fn lane_budget(
    plan: &SearchPlan,
    remaining: SearchExecutionUsage,
    lanes: usize,
    lane: usize,
) -> Option<SearchExecutionBudget> {
    let global = plan.execution_budget().ok()?;
    let max_results = global.max_results().saturating_sub(remaining.results);
    let max_candidates = global.max_candidates().saturating_sub(remaining.candidates);
    let max_work_units = global.max_work_units().saturating_sub(remaining.work_units);
    if max_results == 0 || max_candidates == 0 || max_work_units == 0 {
        return None;
    }
    let remaining_bytes = global
        .max_bytes_read()
        .map(|limit| limit.get().saturating_sub(remaining.bytes_read));
    if remaining_bytes == Some(0) {
        return None;
    }
    let partitioned_bytes = remaining_bytes.map(|limit| partition_allowance(limit, lanes, lane));
    if partitioned_bytes == Some(0) {
        return None;
    }
    let max_bytes = partitioned_bytes.and_then(std::num::NonZeroU64::new);
    SearchExecutionBudget::with_byte_limit(
        partition_allowance(max_results, lanes, lane),
        partition_allowance(max_candidates, lanes, lane),
        partition_allowance(max_work_units, lanes, lane),
        max_bytes,
    )
    .ok()
}

pub(crate) fn remaining_budget(
    plan: &SearchPlan,
    usage: SearchExecutionUsage,
) -> Option<SearchExecutionBudget> {
    let global = plan.execution_budget().ok()?;
    let max_results = global.max_results().saturating_sub(usage.results);
    let max_candidates = global.max_candidates().saturating_sub(usage.candidates);
    let max_work_units = global.max_work_units().saturating_sub(usage.work_units);
    if max_results == 0 || max_candidates == 0 || max_work_units == 0 {
        return None;
    }
    let max_bytes = global
        .max_bytes_read()
        .map(|limit| limit.get().saturating_sub(usage.bytes_read));
    if max_bytes == Some(0) {
        return None;
    }
    SearchExecutionBudget::with_byte_limit(
        max_results,
        max_candidates,
        max_work_units,
        max_bytes.and_then(std::num::NonZeroU64::new),
    )
    .ok()
}
