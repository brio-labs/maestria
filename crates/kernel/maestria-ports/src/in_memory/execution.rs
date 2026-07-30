use crate::{BoundedSearch, PortError};
use maestria_domain::{
    SearchExecution, SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionResource,
    SearchExecutionUsage,
};

pub(crate) struct Meter {
    budget: SearchExecutionBudget,
    usage: SearchExecutionUsage,
}

impl Meter {
    pub(crate) fn new(budget: SearchExecutionBudget) -> Self {
        Self {
            budget,
            usage: SearchExecutionUsage::default(),
        }
    }

    pub(crate) fn candidate(&mut self) -> Option<SearchExecutionResource> {
        if self.usage.candidates >= self.budget.max_candidates() {
            return Some(SearchExecutionResource::Candidates);
        }
        self.usage.candidates = self.usage.candidates.saturating_add(1);
        None
    }

    pub(crate) fn work(&mut self, units: u64) -> Option<SearchExecutionResource> {
        if units
            > self
                .budget
                .max_work_units()
                .saturating_sub(self.usage.work_units)
        {
            return Some(SearchExecutionResource::WorkUnits);
        }
        self.usage.work_units = self.usage.work_units.saturating_add(units);
        None
    }

    pub(crate) fn bytes(&mut self, bytes: u64) -> Option<SearchExecutionResource> {
        let Some(limit) = self.budget.max_bytes_read() else {
            self.usage.bytes_read = self.usage.bytes_read.saturating_add(bytes);
            return None;
        };
        if bytes > limit.get().saturating_sub(self.usage.bytes_read) {
            return Some(SearchExecutionResource::BytesRead);
        }
        self.usage.bytes_read = self.usage.bytes_read.saturating_add(bytes);
        None
    }

    pub(crate) fn result(&mut self) -> Option<SearchExecutionResource> {
        if self.usage.results >= self.budget.max_results() {
            return Some(SearchExecutionResource::Results);
        }
        self.usage.results = self.usage.results.saturating_add(1);
        None
    }

    pub(crate) fn complete<T>(self, hits: Vec<T>) -> BoundedSearch<T> {
        BoundedSearch::new(
            hits,
            SearchExecution::new(self.budget, self.usage, SearchExecutionCompletion::Complete),
        )
    }

    pub(crate) fn exhausted<T>(
        self,
        hits: Vec<T>,
        resource: SearchExecutionResource,
    ) -> BoundedSearch<T> {
        BoundedSearch::new(
            hits,
            SearchExecution::new(
                self.budget,
                self.usage,
                SearchExecutionCompletion::Exhausted(resource),
            ),
        )
    }
}

pub(crate) fn validate_limit(
    limit: usize,
    budget: SearchExecutionBudget,
    context: &'static str,
) -> Result<(), PortError> {
    let limit = u64::try_from(limit).map_err(|_| PortError::InvalidInputContext {
        context,
        source: "result limit does not fit execution budget representation".to_string(),
    })?;
    if limit != budget.max_results() {
        return Err(PortError::InvalidInputContext {
            context,
            source: "query limit and execution budget max_results must agree".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn validate_limit_u32(
    limit: u32,
    budget: SearchExecutionBudget,
    context: &'static str,
) -> Result<(), PortError> {
    if u64::from(limit) != budget.max_results() {
        return Err(PortError::InvalidInputContext {
            context,
            source: "query limit and execution budget max_results must agree".to_string(),
        });
    }
    Ok(())
}

pub(crate) fn saturating_u64(value: usize) -> u64 {
    value as u64
}

pub(crate) fn saturating_usize(value: u64) -> usize {
    value.min(usize::MAX as u64) as usize
}
