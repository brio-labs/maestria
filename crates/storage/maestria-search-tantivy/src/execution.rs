use maestria_domain::{
    SearchExecution, SearchExecutionBudget, SearchExecutionCompletion, SearchExecutionResource,
    SearchExecutionUsage,
};
use maestria_ports::{BoundedSearch, PortError};

pub(crate) struct Meter {
    pub budget: SearchExecutionBudget,
    pub usage: SearchExecutionUsage,
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
            Some(SearchExecutionResource::Candidates)
        } else {
            self.usage.candidates += 1;
            None
        }
    }
    pub(crate) fn work(&mut self, n: u64) -> Option<SearchExecutionResource> {
        if n > self
            .budget
            .max_work_units()
            .saturating_sub(self.usage.work_units)
        {
            Some(SearchExecutionResource::WorkUnits)
        } else {
            self.usage.work_units += n;
            None
        }
    }
    pub(crate) fn bytes(&mut self, n: u64) -> Option<SearchExecutionResource> {
        let Some(limit) = self.budget.max_bytes_read() else {
            self.usage.bytes_read = self.usage.bytes_read.saturating_add(n);
            return None;
        };
        if n > limit.get().saturating_sub(self.usage.bytes_read) {
            Some(SearchExecutionResource::BytesRead)
        } else {
            self.usage.bytes_read += n;
            None
        }
    }
    pub(crate) fn result(&mut self) -> Option<SearchExecutionResource> {
        if self.usage.results >= self.budget.max_results() {
            Some(SearchExecutionResource::Results)
        } else {
            self.usage.results += 1;
            None
        }
    }
    pub(crate) fn done<T>(
        self,
        hits: Vec<T>,
        completion: SearchExecutionCompletion,
    ) -> BoundedSearch<T> {
        BoundedSearch::new(
            hits,
            SearchExecution::new(self.budget, self.usage, completion),
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
