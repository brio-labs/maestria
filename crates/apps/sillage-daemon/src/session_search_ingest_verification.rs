use std::{
    fmt,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use sillage_ports::{
    BoundedParseError, FileHandle, FileHandleRef, FileMetadata, ParseBudget, ParseBudgetFailure,
    ParseCheckpoint, ParseContext, ParseOutcome, ParsedArtifact, Parser, PortError,
};
use sillage_search_tantivy::{MemoryBudgetError, RamMemoryBudget, RamMemoryReservation};
use tokio_util::sync::CancellationToken;

use crate::session_search::{SessionOperationSettlement, deadline::SessionDeadline};

pub(super) struct VerificationParser<'a> {
    parser: &'a sillage_parsers::ParserRegistry,
    memory: &'a Arc<RamMemoryBudget>,
    cancellation: &'a CancellationToken,
    deadline: SessionDeadline,
    max_file_bytes: u64,
    max_work_units: u64,
    work: &'a Mutex<u64>,
    state: Mutex<VerificationState>,
}

#[derive(Default)]
struct VerificationState {
    admission: Option<RamMemoryReservation>,
    memory_failure: Option<MemoryBudgetError>,
    budget_failure: Option<VerificationBudgetFailure>,
}

#[derive(Debug, Clone, Copy)]
enum VerificationBudgetFailure {
    WorkUnits,
    Deadline,
    Cancelled,
    FileBytes,
}

#[derive(Debug)]
pub(in crate::session_search) struct VerificationBudgetExceeded(VerificationBudgetFailure);

impl fmt::Display for VerificationBudgetExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "session snapshot verification exceeded its {:?} budget",
            self.0
        )
    }
}
impl std::error::Error for VerificationBudgetExceeded {}

impl VerificationBudgetExceeded {
    pub(super) fn session_settlement(&self) -> Option<SessionOperationSettlement> {
        match self.0 {
            VerificationBudgetFailure::Cancelled => {
                Some(SessionOperationSettlement::RoutineCancellation)
            }
            VerificationBudgetFailure::Deadline => {
                Some(SessionOperationSettlement::DeadlineExceeded)
            }
            VerificationBudgetFailure::WorkUnits | VerificationBudgetFailure::FileBytes => None,
        }
    }
}

pub(super) fn work_budget_exceeded() -> anyhow::Error {
    VerificationBudgetExceeded(VerificationBudgetFailure::WorkUnits).into()
}

impl<'a> VerificationParser<'a> {
    pub(super) fn new(
        parser: &'a sillage_parsers::ParserRegistry,
        memory: &'a Arc<RamMemoryBudget>,
        cancellation: &'a CancellationToken,
        deadline: SessionDeadline,
        max_file_bytes: u64,
        max_work_units: u64,
        work: &'a Mutex<u64>,
    ) -> Self {
        Self {
            parser,
            memory,
            cancellation,
            deadline,
            max_file_bytes,
            max_work_units,
            work,
            state: Mutex::new(VerificationState::default()),
        }
    }

    pub(super) fn check(&self) -> Result<()> {
        let state = self
            .state
            .lock()
            .map_err(|error| anyhow!("session verification state lock failed: {error}"))?;
        if let Some(error) = state.memory_failure {
            return Err(error.into());
        }
        if let Some(failure) = state.budget_failure {
            let error = VerificationBudgetExceeded(failure);
            return match error.session_settlement() {
                Some(settlement) => Err(anyhow::Error::new(error).context(settlement)),
                None => Err(error.into()),
            };
        }
        Ok(())
    }

    fn parse_failure(
        &self,
        state: &mut VerificationState,
        failure: VerificationBudgetFailure,
    ) -> PortError {
        state.budget_failure = Some(failure);
        PortError::downstream(
            "session verification budget",
            VerificationBudgetExceeded(failure).to_string(),
        )
    }
    fn validate_request(
        &self,
        state: &mut VerificationState,
        work: u64,
        file: &FileHandle,
    ) -> Result<(u64, usize, u64), PortError> {
        if self.cancellation.is_cancelled() {
            return Err(self.parse_failure(state, VerificationBudgetFailure::Cancelled));
        }
        if self.deadline.is_elapsed() {
            return Err(self.parse_failure(state, VerificationBudgetFailure::Deadline));
        }
        let input_bytes = u64::try_from(file.bytes.len()).map_err(|error| {
            PortError::downstream("session verification work", error.to_string())
        })?;
        if input_bytes > self.max_file_bytes {
            return Err(self.parse_failure(state, VerificationBudgetFailure::FileBytes));
        }
        let next_work = work
            .checked_add(input_bytes)
            .ok_or_else(|| self.parse_failure(state, VerificationBudgetFailure::WorkUnits))?;
        if next_work > self.max_work_units {
            return Err(self.parse_failure(state, VerificationBudgetFailure::WorkUnits));
        }
        let path_bytes = file.path.as_os_str().len();
        Ok((input_bytes, path_bytes, next_work))
    }

    fn reserve_parse_memory(
        &self,
        state: &mut VerificationState,
        transient_bytes: u64,
    ) -> Result<(RamMemoryReservation, u64), PortError> {
        let mut admission = match self.memory.reserve(transient_bytes) {
            Ok(admission) => admission,
            Err(error) => {
                let message = error.to_string();
                state.memory_failure = Some(error);
                return Err(PortError::downstream(
                    "session verification admission",
                    message,
                ));
            }
        };
        let output_allowance = self.memory.available_bytes();
        admission
            .resize(transient_bytes.saturating_add(output_allowance))
            .map_err(|error| {
                let message = error.to_string();
                state.memory_failure = Some(error);
                PortError::downstream("session verification admission", message)
            })?;
        Ok((admission, output_allowance))
    }

    fn run_bounded_parse(
        &self,
        file: &FileHandle,
        context: ParseContext,
        max_work_units: u64,
        max_output_bytes: usize,
    ) -> (Result<ParseOutcome, BoundedParseError>, u64, usize) {
        let mut checkpoint = || {
            if self.cancellation.is_cancelled() {
                ParseCheckpoint::Cancelled
            } else if self.deadline.is_elapsed() {
                ParseCheckpoint::DeadlineExceeded
            } else {
                ParseCheckpoint::Continue
            }
        };
        let mut budget = ParseBudget::new(max_work_units, max_output_bytes, &mut checkpoint);
        let parsed = self.parser.parse_outcome_bounded(
            FileHandleRef::new(&file.path, &file.bytes),
            context,
            &mut budget,
        );
        (parsed, budget.work_units_used(), budget.output_bytes_used())
    }

    fn finish_bounded_parse(
        &self,
        state: &mut VerificationState,
        parsed: Result<ParseOutcome, BoundedParseError>,
        output_allowance: u64,
    ) -> Result<ParsedArtifact, PortError> {
        match parsed {
            Ok(ParseOutcome::Complete(parsed)) => Ok(parsed),
            Ok(ParseOutcome::NeedsOcr { .. }) => Err(PortError::InvalidInputContext {
                context: "parser returned OCR pending without a page set",
                source: "OCR-capable parsers must implement parse_outcome".to_string(),
            }),
            Err(BoundedParseError::Unsupported) => Err(PortError::downstream(
                "bounded session verification parse",
                "parser does not support bounded parsing",
            )),
            Err(BoundedParseError::Parse(error)) => Err(error),
            Err(BoundedParseError::Budget(
                ParseBudgetFailure::WorkLimitExceeded { .. }
                | ParseBudgetFailure::WorkChargeTooLarge { .. },
            )) => Err(self.parse_failure(state, VerificationBudgetFailure::WorkUnits)),
            Err(BoundedParseError::Budget(ParseBudgetFailure::OutputLimitExceeded {
                used,
                requested,
                ..
            })) => {
                let failure = match used
                    .checked_add(requested)
                    .and_then(|bytes| u64::try_from(bytes).ok())
                {
                    Some(requested) => MemoryBudgetError::Exhausted {
                        requested,
                        available: output_allowance,
                    },
                    None => MemoryBudgetError::AccountingOverflow,
                };
                let message = failure.to_string();
                state.memory_failure = Some(failure);
                Err(PortError::downstream(
                    "session verification output admission",
                    message,
                ))
            }
            Err(BoundedParseError::Budget(ParseBudgetFailure::Cancelled)) => {
                Err(self.parse_failure(state, VerificationBudgetFailure::Cancelled))
            }
            Err(BoundedParseError::Budget(ParseBudgetFailure::DeadlineExceeded)) => {
                Err(self.parse_failure(state, VerificationBudgetFailure::Deadline))
            }
        }
    }

    fn check_parse_active(&self, state: &mut VerificationState) -> Result<(), PortError> {
        if self.cancellation.is_cancelled() {
            return Err(self.parse_failure(state, VerificationBudgetFailure::Cancelled));
        }
        if self.deadline.is_elapsed() {
            return Err(self.parse_failure(state, VerificationBudgetFailure::Deadline));
        }
        Ok(())
    }
}

impl Parser for VerificationParser<'_> {
    fn id(&self) -> &'static str {
        self.parser.id()
    }

    fn supports(&self, file: &FileMetadata) -> bool {
        self.parser.supports(file)
    }

    fn parse(&self, file: FileHandle, context: ParseContext) -> Result<ParsedArtifact, PortError> {
        let mut state = self.state.lock().map_err(|error| {
            PortError::downstream("session verification state", error.to_string())
        })?;
        let mut work = self.work.lock().map_err(|error| {
            PortError::downstream("session verification work", error.to_string())
        })?;
        let (input_bytes, path_bytes, next_work) =
            self.validate_request(&mut state, *work, &file)?;
        let path_bytes_u64 = u64::try_from(path_bytes).map_err(|error| {
            PortError::downstream("session verification work", error.to_string())
        })?;
        let transient_bytes = input_bytes
            .saturating_add(path_bytes_u64)
            .saturating_add(1024);
        let (mut admission, output_allowance) =
            self.reserve_parse_memory(&mut state, transient_bytes)?;
        let max_work_units = self.max_work_units.saturating_sub(next_work);
        let max_output_bytes = usize::try_from(output_allowance).map_err(|error| {
            PortError::downstream("session verification admission", error.to_string())
        })?;
        let (parsed, work_used, output_used) =
            self.run_bounded_parse(&file, context, max_work_units, max_output_bytes);
        *work = next_work
            .checked_add(work_used)
            .ok_or_else(|| self.parse_failure(&mut state, VerificationBudgetFailure::WorkUnits))?;
        let parsed = self.finish_bounded_parse(&mut state, parsed, output_allowance)?;
        self.check_parse_active(&mut state)?;
        let parsed_bytes =
            super::store::parsed_memory_estimate(&parsed, path_bytes).map_err(|error| {
                PortError::downstream("session verification output admission", error.to_string())
            })?;
        let charged_output = u64::try_from(output_used).map_err(|error| {
            PortError::downstream("session verification output admission", error.to_string())
        })?;
        admission
            .resize(transient_bytes.saturating_add(charged_output.max(parsed_bytes)))
            .map_err(|error| {
                let message = error.to_string();
                state.memory_failure = Some(error);
                PortError::downstream("session verification output admission", message)
            })?;
        state.admission = Some(admission);
        Ok(parsed)
    }
}
