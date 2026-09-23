use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::errors::LauncherError;

#[derive(Serialize)]
struct FrameReadyEvent<'a> {
    event: &'a str,
    generation: u64,
    renderer_ready_ms: f64,
    native_receipt_to_ack_ms: f64,
    presentation: &'static str,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FramePhase {
    Activation,
    Results,
}

#[derive(Default)]
struct PendingFrames {
    initial_activation: Option<i64>,
    activation: Option<(u64, i64)>,
    results: Option<(u64, i64)>,
}

/// Opt-in, content-free timing of native receipt through a renderer frame acknowledgement.
pub struct LauncherMetrics {
    enabled: bool,
    pending: Mutex<PendingFrames>,
}

impl LauncherMetrics {
    pub fn new() -> Self {
        Self {
            enabled: std::env::var_os("MAESTRIA_LAUNCHER_METRICS").as_deref()
                == Some(std::ffi::OsStr::new("1")),
            pending: Mutex::new(PendingFrames::default()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn activation_received(&self, generation: Option<u64>, received_at: i64) {
        if !self.enabled {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.results = None;
            match generation {
                Some(generation) => pending.activation = Some((generation, received_at)),
                None => pending.initial_activation = Some(received_at),
            }
        }
    }

    pub fn initial_frame_requested(&self, generation: u64) {
        if !self.enabled {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            pending.results = None;
            let received_at = match pending.initial_activation.take() {
                Some(received_at) => received_at,
                None => glib::monotonic_time(),
            };
            pending.activation = Some((generation, received_at));
        }
    }

    pub fn results_requested(&self, generation: u64) {
        if !self.enabled {
            return;
        }
        if let Ok(mut pending) = self.pending.lock() {
            match pending.results {
                Some((current, _)) if current > generation => {}
                _ => pending.results = Some((generation, glib::monotonic_time())),
            }
        }
    }

    pub fn frame_ready(
        &self,
        generation: u64,
        phase: FramePhase,
        renderer_elapsed_ms: f64,
    ) -> Result<(), LauncherError> {
        if !self.enabled {
            return Err(LauncherError::invalid_request(
                "Launcher metrics are disabled",
            ));
        }
        if !renderer_elapsed_ms.is_finite() || !(0.0..=60_000.0).contains(&renderer_elapsed_ms) {
            return Err(LauncherError::invalid_request("Invalid renderer timing"));
        }
        let (event, received_at) = {
            let mut pending = self.pending.lock().map_err(|_| {
                LauncherError::platform_unavailable("Launcher timing state is unavailable")
            })?;
            let (slot, event) = match phase {
                FramePhase::Activation => (&mut pending.activation, "launcher_presented"),
                FramePhase::Results => (&mut pending.results, "launcher_results_presented"),
            };
            match slot.take() {
                Some((current, received_at)) if current == generation => (event, received_at),
                Some(current) => {
                    *slot = Some(current);
                    return Err(LauncherError::stale_result(
                        "This frame is no longer current",
                    ));
                }
                None => {
                    return Err(LauncherError::invalid_request(
                        "No frame is awaiting acknowledgement",
                    ));
                }
            }
        };
        let native_elapsed_ms = (glib::monotonic_time() - received_at).max(0) as f64 / 1000.0;
        let event = FrameReadyEvent {
            event,
            generation,
            renderer_ready_ms: renderer_elapsed_ms,
            native_receipt_to_ack_ms: native_elapsed_ms,
            presentation: "renderer_ready_not_physical_pixels",
        };
        match serde_json::to_string(&event) {
            Ok(payload) => eprintln!("{payload}"),
            Err(error) => eprintln!("Launcher metrics serialization failed: {error}"),
        }
        Ok(())
    }
}

impl Default for LauncherMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_and_duplicate_frames_cannot_consume_newer_measurements() {
        let metrics = LauncherMetrics {
            enabled: true,
            pending: Mutex::new(PendingFrames::default()),
        };
        metrics.activation_received(Some(5), glib::monotonic_time());
        metrics.activation_received(Some(6), glib::monotonic_time());
        let stale = metrics.frame_ready(5, FramePhase::Activation, 1.0);
        assert_eq!(
            stale.err().map(|error| error.code).as_deref(),
            Some("stale_result")
        );
        assert!(metrics.frame_ready(6, FramePhase::Activation, 1.0).is_ok());
        assert!(metrics.frame_ready(6, FramePhase::Activation, 1.0).is_err());

        metrics.results_requested(7);
        metrics.activation_received(Some(8), glib::monotonic_time());
        assert!(metrics.frame_ready(7, FramePhase::Results, 1.0).is_err());
    }

    #[test]
    fn stale_results_requests_cannot_replace_newer_measurements() {
        let metrics = LauncherMetrics {
            enabled: true,
            pending: Mutex::new(PendingFrames::default()),
        };

        metrics.results_requested(8);
        metrics.results_requested(7);

        assert_eq!(
            metrics
                .frame_ready(7, FramePhase::Results, 1.0)
                .err()
                .map(|error| error.code)
                .as_deref(),
            Some("stale_result")
        );
        assert!(metrics.frame_ready(8, FramePhase::Results, 1.0).is_ok());
    }

    #[test]
    fn disabled_and_nonfinite_metrics_are_rejected() {
        let disabled = LauncherMetrics {
            enabled: false,
            pending: Mutex::new(PendingFrames::default()),
        };
        assert!(disabled.frame_ready(1, FramePhase::Results, 1.0).is_err());
        let metrics = LauncherMetrics {
            enabled: true,
            pending: Mutex::new(PendingFrames::default()),
        };
        metrics.results_requested(1);
        assert!(
            metrics
                .frame_ready(1, FramePhase::Results, f64::NAN)
                .is_err()
        );
        assert!(metrics.frame_ready(1, FramePhase::Results, 1.0).is_ok());
    }
}
