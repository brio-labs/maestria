pub(super) fn search_budget(
    limit: u64,
) -> Result<maestria_domain::SearchExecutionBudget, maestria_domain::SearchCompatibilityError> {
    maestria_domain::SearchExecutionBudget::new(limit, 10_000, 100_000, 0)
}
