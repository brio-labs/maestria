use std::{env, ffi::OsString, path::PathBuf, time::Duration};

use thiserror::Error;

pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MIN_TIMEOUT: Duration = Duration::from_millis(100);
pub(crate) const MAX_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub(crate) struct WorkerArguments {
    pub(crate) bundle_root: PathBuf,
    pub(crate) entrypoint_id: String,
    pub(crate) timeout: Duration,
}

#[derive(Debug, Error)]
pub(crate) enum ArgumentError {
    #[error("missing required option {0}")]
    Missing(&'static str),
    #[error("option {0} requires a value")]
    MissingValue(&'static str),
    #[error("unknown or duplicate option")]
    UnknownOrDuplicate,
    #[error("option value must be valid UTF-8")]
    InvalidEncoding,
    #[error("timeout must be an integer between 100 and 300000 milliseconds")]
    InvalidTimeout,
}

impl WorkerArguments {
    pub(crate) fn parse() -> Result<Self, ArgumentError> {
        let mut bundle_root = None;
        let mut entrypoint_id = None;
        let mut timeout = DEFAULT_TIMEOUT;
        let mut timeout_seen = false;
        let mut args = env::args_os().skip(1);

        while let Some(option) = args.next() {
            match option.to_str() {
                Some("--bundle-root") => {
                    if bundle_root.is_some() {
                        return Err(ArgumentError::UnknownOrDuplicate);
                    }
                    bundle_root = Some(PathBuf::from(next_value(&mut args, "--bundle-root")?));
                }
                Some("--entrypoint-id") => {
                    if entrypoint_id.is_some() {
                        return Err(ArgumentError::UnknownOrDuplicate);
                    }
                    entrypoint_id = Some(string_value(next_value(&mut args, "--entrypoint-id")?)?);
                }
                Some("--timeout-ms") => {
                    if timeout_seen {
                        return Err(ArgumentError::UnknownOrDuplicate);
                    }
                    timeout_seen = true;
                    let timeout_text = string_value(next_value(&mut args, "--timeout-ms")?)?;
                    let timeout_ms = timeout_text
                        .parse::<u64>()
                        .map_err(|_| ArgumentError::InvalidTimeout)?;
                    timeout = Duration::from_millis(timeout_ms);
                    if !(MIN_TIMEOUT..=MAX_TIMEOUT).contains(&timeout) {
                        return Err(ArgumentError::InvalidTimeout);
                    }
                }
                _ => return Err(ArgumentError::UnknownOrDuplicate),
            }
        }

        Ok(Self {
            bundle_root: bundle_root.ok_or(ArgumentError::Missing("--bundle-root"))?,
            entrypoint_id: entrypoint_id.ok_or(ArgumentError::Missing("--entrypoint-id"))?,
            timeout,
        })
    }
}

fn next_value(
    args: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<OsString, ArgumentError> {
    args.next().ok_or(ArgumentError::MissingValue(option))
}

fn string_value(value: OsString) -> Result<String, ArgumentError> {
    value
        .into_string()
        .map_err(|_| ArgumentError::InvalidEncoding)
}
