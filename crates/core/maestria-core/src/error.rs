use std::fmt;

use maestria_ports::PortError;

#[derive(Debug)]
pub enum CoreError {
    InvalidInput { message: String },
    InvalidEvidence { evidence_id: String, reason: String },
    InvalidManifest { key: String, reason: String },
    NotFound { message: String },
    NotFoundEntity { kind: &'static str, id: String },
    NotAvailable { kind: &'static str, reason: &'static str },
    SearchPlan(maestria_governance::SearchPlanValidationError),
    Port(PortError),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::InvalidEvidence { evidence_id, reason } => {
                write!(f, "invalid evidence {evidence_id}: {reason}")
            }
            Self::InvalidManifest { key, reason } => {
                write!(f, "invalid manifest key '{key}': {reason}")
            }
            Self::SearchPlan(error) => write!(f, "search plan rejected: {error}"),
            Self::NotFound { message } => write!(f, "not found: {message}"),
            Self::NotFoundEntity { kind, id } => write!(f, "not found: {kind} {id}"),
            Self::NotAvailable { kind, reason } => {
                write!(f, "{kind} is not available: {reason}")
            }
            Self::Port(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CoreError {}

impl From<PortError> for CoreError {
    fn from(value: PortError) -> Self {
        Self::Port(value)
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
