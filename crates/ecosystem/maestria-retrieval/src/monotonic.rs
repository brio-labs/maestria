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

    /// Return an instant `duration` before this one, clamped to the epoch.
    ///
    /// Used by interval reporters to seed a "last report" instant in the
    /// past without depending on a wall clock.
    pub fn saturating_sub(self, duration: std::time::Duration) -> Self {
        match self.0.checked_sub(duration) {
            Some(earlier) => Self(earlier),
            None => self,
        }
    }

    /// Return the duration between `earlier` and this instant, clamped to
    /// zero when `earlier` is in the future.
    pub fn duration_since(self, earlier: Self) -> std::time::Duration {
        self.0.saturating_duration_since(earlier.0)
    }
}
