//! Typed errors for repository code intelligence indexing.

use std::fmt;

/// Errors produced by code intelligence indexing operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeIntelError {
    /// External command execution failed.
    Command {
        command: String,
        status: Option<i32>,
        details: String,
    },
    /// I/O operation failed.
    Io {
        operation: String,
        path: String,
        details: String,
    },
    /// Parsing failure for metadata or Rust source.
    Parse { context: String, details: String },
    /// JSON persistence load or save failed.
    Persist { context: String, details: String },
    /// Repository identity derivation failed.
    Identity { context: String, details: String },
    /// Regex construction failed.
    Regex { pattern: String, details: String },
}

impl fmt::Display for CodeIntelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command {
                command,
                status,
                details,
            } => match status {
                Some(status) => write!(f, "command `{command}` failed ({status}): {details}"),
                None => write!(f, "command `{command}` failed to start: {details}"),
            },
            Self::Io {
                operation,
                path,
                details,
            } => write!(f, "{operation} failed for {path}: {details}"),
            Self::Parse { context, details } => {
                write!(f, "parse failure in {context}: {details}")
            }
            Self::Persist { context, details } => {
                write!(f, "persistence failure in {context}: {details}")
            }
            Self::Identity { context, details } => {
                write!(f, "identity discovery failed in {context}: {details}")
            }
            Self::Regex { pattern, details } => {
                write!(f, "invalid regex `{pattern}`: {details}")
            }
        }
    }
}

impl std::error::Error for CodeIntelError {}
