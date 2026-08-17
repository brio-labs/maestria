//! Argument value parsers for the federated realm commands.

/// Bounded result-count parser for `--max-results`.
pub(super) fn parse_federated_results(input: &str) -> Result<usize, &'static str> {
    let value = input
        .parse::<usize>()
        .map_err(|_| "max results must be a number")?;
    if !(1..=100).contains(&value) {
        return Err("maximum results must be 1..=100");
    }
    Ok(value)
}

/// Bounded evidence-bytes parser for `--max-evidence-bytes`.
pub(super) fn parse_federated_evidence_bytes(input: &str) -> Result<usize, &'static str> {
    let value = input
        .parse::<usize>()
        .map_err(|_| "max evidence bytes must be a number")?;
    if !(1..=65_536).contains(&value) {
        return Err("maximum evidence bytes must be 1..=65536");
    }
    Ok(value)
}
