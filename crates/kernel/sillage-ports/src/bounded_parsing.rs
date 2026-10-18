#![forbid(unsafe_code)]

use crate::PortError;

/// Cooperative result returned by a bounded parser's cancellation/deadline check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseCheckpoint {
    Continue,
    Cancelled,
    DeadlineExceeded,
}

/// Typed reason a bounded parse could not satisfy its work/output or checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseBudgetFailure {
    WorkLimitExceeded {
        limit: u64,
        used: u64,
        requested: u64,
    },
    WorkChargeTooLarge {
        max: u64,
        requested: u64,
    },
    OutputLimitExceeded {
        limit: usize,
        used: usize,
        requested: usize,
    },
    Cancelled,
    DeadlineExceeded,
}

impl std::fmt::Display for ParseBudgetFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkLimitExceeded {
                limit,
                used,
                requested,
            } => write!(
                formatter,
                "parse work limit exceeded (limit {limit}, used {used}, requested {requested})"
            ),
            Self::WorkChargeTooLarge { max, requested } => write!(
                formatter,
                "parse work charge exceeds checkpoint interval (max {max}, requested {requested})"
            ),
            Self::OutputLimitExceeded {
                limit,
                used,
                requested,
            } => write!(
                formatter,
                "parse output limit exceeded (limit {limit}, used {used}, requested {requested})"
            ),
            Self::Cancelled => formatter.write_str("parse cancelled"),
            Self::DeadlineExceeded => formatter.write_str("parse deadline exceeded"),
        }
    }
}

impl std::error::Error for ParseBudgetFailure {}

/// Per-parse logical work/output quota and cooperative checkpoint callback.
///
/// Output bytes are admission accounting for parser-owned output, not an RSS or
/// physical-memory measurement. Work units are parser-defined logical charges.
pub struct ParseBudget<'a> {
    max_work_units: u64,
    max_output_bytes: usize,
    work_units_used: u64,
    output_bytes_used: usize,
    work_since_checkpoint: u64,
    checkpoint: &'a mut dyn FnMut() -> ParseCheckpoint,
}

impl<'a> ParseBudget<'a> {
    /// Maximum work units charged between automatic checkpoints.
    pub const CHECKPOINT_INTERVAL: u64 = 1024;

    /// Creates a budget whose callback is checked at explicit checkpoints and
    /// after each checkpoint interval of charged work.
    pub fn new(
        max_work_units: u64,
        max_output_bytes: usize,
        checkpoint: &'a mut dyn FnMut() -> ParseCheckpoint,
    ) -> Self {
        Self {
            max_work_units,
            max_output_bytes,
            work_units_used: 0,
            output_bytes_used: 0,
            work_since_checkpoint: 0,
            checkpoint,
        }
    }

    /// Checks cancellation/deadline state and resets the automatic work interval.
    pub fn checkpoint(&mut self) -> Result<(), ParseBudgetFailure> {
        match (self.checkpoint)() {
            ParseCheckpoint::Continue => {
                self.work_since_checkpoint = 0;
                Ok(())
            }
            ParseCheckpoint::Cancelled => Err(ParseBudgetFailure::Cancelled),
            ParseCheckpoint::DeadlineExceeded => Err(ParseBudgetFailure::DeadlineExceeded),
        }
    }

    /// Charges logical work; a single charge cannot exceed the checkpoint interval.
    pub fn charge_work(&mut self, units: u64) -> Result<(), ParseBudgetFailure> {
        if units > Self::CHECKPOINT_INTERVAL {
            return Err(ParseBudgetFailure::WorkChargeTooLarge {
                max: Self::CHECKPOINT_INTERVAL,
                requested: units,
            });
        }
        let next_used = self.work_units_used.checked_add(units).ok_or(
            ParseBudgetFailure::WorkLimitExceeded {
                limit: self.max_work_units,
                used: self.work_units_used,
                requested: units,
            },
        )?;
        if next_used > self.max_work_units {
            return Err(ParseBudgetFailure::WorkLimitExceeded {
                limit: self.max_work_units,
                used: self.work_units_used,
                requested: units,
            });
        }
        self.work_units_used = next_used;
        self.work_since_checkpoint += units;
        if self.work_since_checkpoint >= Self::CHECKPOINT_INTERVAL {
            self.work_since_checkpoint -= Self::CHECKPOINT_INTERVAL;
            match (self.checkpoint)() {
                ParseCheckpoint::Continue => {}
                ParseCheckpoint::Cancelled => return Err(ParseBudgetFailure::Cancelled),
                ParseCheckpoint::DeadlineExceeded => {
                    return Err(ParseBudgetFailure::DeadlineExceeded);
                }
            }
        }
        Ok(())
    }

    /// Admits logical parser-owned output bytes before the associated allocation.
    pub fn charge_output(&mut self, bytes: usize) -> Result<(), ParseBudgetFailure> {
        let next_used = self.output_bytes_used.checked_add(bytes).ok_or(
            ParseBudgetFailure::OutputLimitExceeded {
                limit: self.max_output_bytes,
                used: self.output_bytes_used,
                requested: bytes,
            },
        )?;
        if next_used > self.max_output_bytes {
            return Err(ParseBudgetFailure::OutputLimitExceeded {
                limit: self.max_output_bytes,
                used: self.output_bytes_used,
                requested: bytes,
            });
        }
        self.output_bytes_used = next_used;
        Ok(())
    }

    pub const fn work_units_used(&self) -> u64 {
        self.work_units_used
    }

    pub const fn output_bytes_used(&self) -> usize {
        self.output_bytes_used
    }
}

/// Typed bounded-parse refusal, distinguishing unsupported routes from parse and
/// resource/checkpoint failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedParseError {
    Unsupported,
    Parse(PortError),
    Budget(ParseBudgetFailure),
}

impl std::fmt::Display for BoundedParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => formatter.write_str("parser does not support bounded parsing"),
            Self::Parse(error) => error.fmt(formatter),
            Self::Budget(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for BoundedParseError {}

impl From<PortError> for BoundedParseError {
    fn from(error: PortError) -> Self {
        Self::Parse(error)
    }
}

impl From<ParseBudgetFailure> for BoundedParseError {
    fn from(error: ParseBudgetFailure) -> Self {
        Self::Budget(error)
    }
}
