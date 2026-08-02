/// Task status state machine: the status enum and its transition policy.
///
/// Owns the transition graph (`can_transition_to`), completion detection, and
/// the validation path computation reused by application entry points (R28),
/// so lifecycle policy has one owner instead of being re-encoded per caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Draft,
    Open,
    Active,
    Validating,
    Blocked,
    CompletedVerified,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn can_transition_to(self, next: Self) -> bool {
        match self {
            Self::Draft => matches!(next, Self::Open | Self::Cancelled),
            Self::Open => matches!(next, Self::Active | Self::Cancelled),
            Self::Active => matches!(
                next,
                Self::Validating | Self::Blocked | Self::Failed | Self::Cancelled
            ),
            Self::Validating => matches!(
                next,
                Self::CompletedVerified | Self::CompletedWithWarnings | Self::Failed | Self::Active
            ),
            Self::Blocked => matches!(next, Self::Active | Self::Failed | Self::Cancelled),
            Self::CompletedVerified
            | Self::CompletedWithWarnings
            | Self::Failed
            | Self::Cancelled => false,
        }
    }

    pub fn is_completion(self) -> bool {
        matches!(self, Self::CompletedVerified | Self::CompletedWithWarnings)
    }

    /// Ordered list of statuses (excluding the start) to walk through to reach
    /// `Validating` along the transition graph, `Some(vec![])` when already
    /// validating, and `None` when unreachable. Owns the transition-path
    /// policy that application entry points reuse (R28).
    pub fn path_to_validating(self) -> Option<Vec<Self>> {
        if self == Self::Validating {
            return Some(Vec::new());
        }
        let mut queue = std::collections::VecDeque::from([(self, Vec::<Self>::new())]);
        let mut visited = vec![self];
        while let Some((status, path)) = queue.pop_front() {
            for next in [
                Self::Draft,
                Self::Open,
                Self::Active,
                Self::Validating,
                Self::Blocked,
                Self::CompletedVerified,
                Self::CompletedWithWarnings,
                Self::Failed,
                Self::Cancelled,
            ] {
                if !status.can_transition_to(next) || visited.contains(&next) {
                    continue;
                }
                visited.push(next);
                let mut extended = path.clone();
                extended.push(next);
                if next == Self::Validating {
                    return Some(extended);
                }
                queue.push_back((next, extended));
            }
        }
        None
    }
}
