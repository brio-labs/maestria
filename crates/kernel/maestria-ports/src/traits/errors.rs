use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    NotFound,
    Conflict {
        message: String,
    },
    InvalidInput {
        message: String,
    },
    Downstream {
        message: String,
    },
    Internal {
        message: String,
    },
    InternalContext {
        context: &'static str,
        source: String,
    },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "not found"),
            Self::Conflict { message } => write!(f, "conflict: {message}"),
            Self::InvalidInput { message } => write!(f, "invalid input: {message}"),
            Self::Downstream { message } => write!(f, "downstream error: {message}"),
            Self::Internal { message } => write!(f, "internal error: {message}"),
            Self::InternalContext { context, source } => {
                write!(f, "internal error ({context}): {source}")
            }
        }
    }
}

impl std::error::Error for PortError {}
