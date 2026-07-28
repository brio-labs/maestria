/// Monotonic timestamp used for retrieval latency accounting.
#[derive(Clone, Copy)]
pub struct MonotonicInstant(tokio::time::Instant);

impl MonotonicInstant {
    /// Capture the current monotonic instant.
    pub fn now() -> Self {
        Self(tokio::time::Instant::now())
    }

    /// Return the elapsed duration since this instant.
    pub fn elapsed(self) -> std::time::Duration {
        self.0.elapsed()
    }
}
