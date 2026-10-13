use std::path::Path;

use anyhow::Result;
use sillage_ports::{
    BoundedParseError, FileHandleRef, ParseBudget, ParseBudgetFailure, ParseCheckpoint,
    ParseContext, ParseOutcome, Parser,
};
use sillage_search_tantivy::{MemoryBudgetError, RamMemoryReservation};
use tokio_util::sync::CancellationToken;

use super::super::SessionBuilder;
use super::super::ensure_not_cancelled;

impl SessionBuilder<'_> {
    pub(super) fn parse_source(
        &mut self,
        path: &Path,
        input: &sillage_domain::ArtifactDetected,
        cancellation: &CancellationToken,
    ) -> Result<Option<(sillage_ports::ParsedArtifact, RamMemoryReservation)>> {
        ensure_not_cancelled(cancellation)?;
        if self.deadline_reached() {
            self.defer();
            return Ok(None);
        }
        let raw_bytes = u64::try_from(path.as_os_str().len())?.saturating_add(1024);
        let Some((admission, output_allowance)) = self.reserve_parse_memory(raw_bytes)? else {
            return Ok(None);
        };
        let max_work_units = self
            .choice
            .budget()
            .max_work_units()
            .saturating_sub(self.work_units);
        let max_output_bytes = usize::try_from(output_allowance)?;
        let (parsed, work_used, output_used) =
            self.run_bounded_parse(path, input, cancellation, max_work_units, max_output_bytes);
        self.work_units = self.work_units.saturating_add(work_used);
        self.finish_bounded_parse(
            path,
            parsed,
            output_used,
            raw_bytes,
            admission,
            cancellation,
        )
    }

    fn reserve_parse_memory(
        &mut self,
        raw_bytes: u64,
    ) -> Result<Option<(RamMemoryReservation, u64)>> {
        let mut admission = match self.stores.memory.reserve(raw_bytes) {
            Ok(reservation) => reservation,
            Err(MemoryBudgetError::Exhausted { .. }) => {
                self.defer();
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let output_allowance = self.stores.memory.available_bytes();
        admission.resize(raw_bytes.saturating_add(output_allowance))?;
        Ok(Some((admission, output_allowance)))
    }

    fn run_bounded_parse(
        &self,
        path: &Path,
        input: &sillage_domain::ArtifactDetected,
        cancellation: &CancellationToken,
        max_work_units: u64,
        max_output_bytes: usize,
    ) -> (Result<ParseOutcome, BoundedParseError>, u64, usize) {
        let mut checkpoint = || {
            #[cfg(test)]
            tests::pause_before_parse_checkpoint(path);
            if cancellation.is_cancelled() {
                ParseCheckpoint::Cancelled
            } else if self.deadline_reached() {
                ParseCheckpoint::DeadlineExceeded
            } else {
                ParseCheckpoint::Continue
            }
        };
        let mut parse_budget = ParseBudget::new(max_work_units, max_output_bytes, &mut checkpoint);
        let parsed = self.stores.parser.parse_outcome_bounded(
            FileHandleRef::new(path, &input.source_bytes),
            ParseContext {
                artifact_id: input.artifact_id,
            },
            &mut parse_budget,
        );
        (
            parsed,
            parse_budget.work_units_used(),
            parse_budget.output_bytes_used(),
        )
    }

    fn finish_bounded_parse(
        &mut self,
        path: &Path,
        parsed: Result<ParseOutcome, BoundedParseError>,
        output_used: usize,
        raw_bytes: u64,
        admission: RamMemoryReservation,
        cancellation: &CancellationToken,
    ) -> Result<Option<(sillage_ports::ParsedArtifact, RamMemoryReservation)>> {
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(BoundedParseError::Unsupported) => {
                self.unsupported();
                return Ok(None);
            }
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::WorkLimitExceeded { .. }
                | ParseBudgetFailure::WorkChargeTooLarge { .. }
                | ParseBudgetFailure::OutputLimitExceeded { .. },
            )) => {
                self.defer();
                return Ok(None);
            }
            Err(error @ BoundedParseError::Budget(ParseBudgetFailure::Cancelled)) => {
                return Err(anyhow::Error::new(error)
                    .context(crate::SessionOperationSettlement::RoutineCancellation));
            }
            Err(error @ BoundedParseError::Budget(ParseBudgetFailure::DeadlineExceeded)) => {
                return Err(anyhow::Error::new(error)
                    .context(crate::SessionOperationSettlement::DeadlineExceeded));
            }
            Err(error) => return Err(error.into()),
        };
        ensure_not_cancelled(cancellation)?;
        if self.deadline_reached() {
            self.defer();
            return Ok(None);
        }
        match parsed {
            ParseOutcome::Complete(parsed) if parsed.chunks.is_empty() => {
                self.unsupported();
                Ok(None)
            }
            ParseOutcome::Complete(parsed) => {
                let parsed_bytes =
                    super::super::store::parsed_memory_estimate(&parsed, path.as_os_str().len())?;
                let charged_output = u64::try_from(output_used)?;
                let retained_bytes = charged_output.max(parsed_bytes);
                let mut admission = admission;
                admission.resize(raw_bytes.saturating_add(retained_bytes))?;
                Ok(Some((parsed, admission)))
            }
            ParseOutcome::NeedsOcr { .. } => {
                self.coverage.needs_ocr_sources = self.coverage.needs_ocr_sources.saturating_add(1);
                self.coverage.coverage_complete = false;
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
#[path = "session_search_ingest_parse_tests.rs"]
mod tests;
#[cfg(test)]
pub(in crate::session_search) use tests::{
    PARSE_TEST_SERIAL, ParseCheckpointGate, clear_gate, install_gate,
};
