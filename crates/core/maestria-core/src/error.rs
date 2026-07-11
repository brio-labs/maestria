use std::fmt;

use maestria_ports::PortError;

#[derive(Debug)]
pub enum CoreError {
    InvalidInput { message: String },
    NotFound { message: String },
    Port(PortError),
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::NotFound { message } => write!(f, "not found: {message}"),
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
