//! Local shell harness adapter.
//!
//! Responsibility map:
//! - `adapter`: port adapter and request execution.
//! - `command`: command and path validation.
//! - `process`: trusted process spawning and bounded output collection.
//! - `tokenize`: restricted command tokenization.

mod adapter;
mod command;
mod process;
mod tokenize;
pub use adapter::LocalShellHarnessAdapter;

#[cfg(test)]
mod test_helpers;

#[cfg(test)]
mod tests_success;

#[cfg(test)]
mod tests_nonzero_exit;

#[cfg(test)]
mod tests_rejected_grammar;

#[cfg(test)]
mod tests_rejected_path;

#[cfg(test)]
mod tests_process;

#[cfg(test)]
mod tests_timeout;

#[cfg(test)]
mod tests_cancellation;

#[cfg(test)]
mod tests_capabilities;

#[cfg(test)]
mod tests_filename_pattern;

#[cfg(test)]
mod tests_contract;

#[cfg(test)]
mod tests_boundary;
