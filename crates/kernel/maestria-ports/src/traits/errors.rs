use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    NotFound,
    Conflict {
        message: String,
    },
    InvalidInputContext {
        context: &'static str,
        source: String,
    },
    Downstream {
        message: String,
    },
    DownstreamContext {
        context: &'static str,
        source: String,
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
            Self::InvalidInputContext { context, source } => {
                write!(f, "invalid input ({context}): {source}")
            }
            Self::Downstream { message } => write!(f, "downstream error: {message}"),
            Self::DownstreamContext { context, source } => {
                write!(f, "downstream error ({context}): {source}")
            }
            Self::InternalContext { context, source } => {
                write!(f, "internal error ({context}): {source}")
            }
        }
    }
}

impl PortError {
    pub fn invalid_input(context: &'static str, source: impl Into<String>) -> Self {
        Self::InvalidInputContext {
            context,
            source: source.into(),
        }
    }

    pub fn downstream(context: &'static str, source: impl Into<String>) -> Self {
        Self::DownstreamContext {
            context,
            source: source.into(),
        }
    }

    pub fn internal(context: &'static str, source: impl Into<String>) -> Self {
        Self::InternalContext {
            context,
            source: source.into(),
        }
    }
}

impl PortError {
    pub fn is_invalid_input(&self) -> bool {
        matches!(self, Self::InvalidInputContext { .. })
    }

    pub fn is_downstream(&self) -> bool {
        matches!(
            self,
            Self::Downstream { .. } | Self::DownstreamContext { .. }
        )
    }

    pub fn is_internal(&self) -> bool {
        matches!(self, Self::InternalContext { .. })
    }

    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::NotFound)
    }

    pub fn is_conflict(&self) -> bool {
        matches!(self, Self::Conflict { .. })
    }
}

impl std::error::Error for PortError {}
