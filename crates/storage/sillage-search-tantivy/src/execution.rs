pub(crate) use sillage_ports::execution::{Meter, validate_limit};

use sillage_domain::{SearchExecutionCompletion, SearchExecutionResource};

pub(crate) fn budget_usize(value: u64) -> usize {
    sillage_domain::saturating_usize(value)
}

pub(crate) fn finish_results(
    meter: &mut Meter,
    count: usize,
    mut stopped: Option<SearchExecutionResource>,
) -> SearchExecutionCompletion {
    for _ in 0..count {
        if let Some(resource) = meter.result() {
            stopped = Some(resource);
            break;
        }
    }
    stopped.map_or(
        SearchExecutionCompletion::Complete,
        SearchExecutionCompletion::Exhausted,
    )
}
