use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use maestria_ports::{PortError, ProviderCallControl};

use crate::MonotonicInstant;

/// Cancellation state owned by one retrieval execution.
#[derive(Debug, Default)]
pub struct SearchCancellation {
    cancelled: AtomicBool,
}

impl SearchCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn control(
        self: &Arc<Self>,
        started: MonotonicInstant,
        max_latency_ms: u32,
        max_response_bytes: usize,
    ) -> SearchCallControl {
        SearchCallControl {
            cancellation: Arc::clone(self),
            started,
            max_latency_ms,
            max_response_bytes,
        }
    }
}

/// Kernel-safe view of a retrieval deadline and cancellation state.
#[derive(Clone)]
pub struct SearchCallControl {
    cancellation: Arc<SearchCancellation>,
    started: MonotonicInstant,
    max_latency_ms: u32,
    max_response_bytes: usize,
}

impl SearchCallControl {
    pub fn check(&self) -> Result<(), PortError> {
        if self.is_cancelled() {
            return Err(PortError::internal(
                "retrieval call control",
                "request cancelled",
            ));
        }
        if self.remaining_ms() == 0 {
            return Err(PortError::downstream(
                "retrieval call control",
                "request deadline elapsed",
            ));
        }
        Ok(())
    }
}

impl ProviderCallControl for SearchCallControl {
    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    fn remaining_ms(&self) -> u32 {
        let elapsed = self.started.elapsed();
        let budget = Duration::from_millis(u64::from(self.max_latency_ms));
        if elapsed >= budget {
            return 0;
        }
        budget
            .saturating_sub(elapsed)
            .as_millis()
            .min(u128::from(u32::MAX)) as u32
    }

    fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }
}

/// Guard used by async-to-blocking boundaries to cancel abandoned work.
pub struct SearchCancellationGuard {
    cancellation: Arc<SearchCancellation>,
}

impl SearchCancellationGuard {
    pub fn new(cancellation: Arc<SearchCancellation>) -> Self {
        Self { cancellation }
    }

    pub fn cancellation(&self) -> &Arc<SearchCancellation> {
        &self.cancellation
    }
}

impl Drop for SearchCancellationGuard {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
