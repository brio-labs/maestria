use std::time::Duration;

use sillage_retrieval::MonotonicInstant;

#[derive(Clone, Copy)]
pub(super) struct SessionDeadline {
    started: MonotonicInstant,
    duration: Duration,
}

impl SessionDeadline {
    pub(super) fn after(duration: Duration) -> Self {
        Self {
            started: MonotonicInstant::now(),
            duration,
        }
    }

    pub(super) fn remaining(self) -> Duration {
        self.duration
            .saturating_sub(MonotonicInstant::now().duration_since(self.started))
    }

    pub(super) fn is_elapsed(self) -> bool {
        self.remaining().is_zero()
    }
}
