#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectFailure {
    Denied(String),
    RequiresApproval(String),
    Failed(String),
    Degraded(String),
}

impl EffectFailure {
    pub(crate) fn retryable(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

impl std::fmt::Display for EffectFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(reason) => write!(formatter, "effect denied: {reason}"),
            Self::RequiresApproval(reason) => {
                write!(formatter, "effect requires approval: {reason}")
            }
            Self::Failed(reason) => write!(formatter, "effect failed: {reason}"),
            Self::Degraded(reason) => write!(formatter, "effect degraded: {reason}"),
        }
    }
}

pub(crate) fn handler_result(
    success: bool,
    effect_name: &'static str,
) -> Result<(), EffectFailure> {
    if success {
        Ok(())
    } else {
        Err(EffectFailure::Failed(effect_name.to_string()))
    }
}
