//! Live progress metrics for the `index` command.
//!
//! Per-file counters with elapsed time and throughput, plus a best-effort
//! embedding row count read from the vector projection while the session
//! writer commits (WAL allows concurrent readers).

use anyhow::Context as _;
use std::time::Duration;

use maestria_core::InstanceLayout;
use maestria_core::{format_duration, rate_per_second};
use maestria_retrieval::MonotonicInstant;
use maestria_vector_sqlite::SqliteVectorIndex;
/// Status lines are emitted at most every five seconds during a run.
const STATUS_REPORT_INTERVAL: Duration = Duration::from_secs(5);

/// Format a byte count in binary units with one decimal (`12.4 KiB`).
pub(crate) fn human_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KIB * KIB * KIB {
        format!("{:.1} GiB", value / (KIB * KIB * KIB))
    } else if value >= KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn row_count_or(index: &Option<SqliteVectorIndex>, fallback: u64) -> u64 {
    let Some(index) = index else {
        return fallback;
    };
    match index.embedding_row_count() {
        Ok(count) => count,
        Err(_) => fallback,
    }
}

/// Best-effort live count of persisted vector rows.
pub(crate) struct ProjectionObserver {
    index: Option<SqliteVectorIndex>,
    initial: u64,
    last: u64,
}

impl ProjectionObserver {
    /// Open the instance projection for counting; returns an error when the
    /// existing projection is corrupt so file-progress does not mask storage
    /// failure (absent projection falls back to file metrics only).
    pub(crate) fn open(layout: &InstanceLayout) -> anyhow::Result<Self> {
        let path = layout.vector_index_dir.join("projection.db");
        if !path.exists() {
            return Ok(Self {
                index: None,
                initial: 0,
                last: 0,
            });
        }
        let index = SqliteVectorIndex::open(&path)
            .with_context(|| format!("open vector projection {}", path.display()))?;
        let initial = match index.embedding_row_count() {
            Ok(count) => count,
            Err(error) => {
                tracing::debug!("failed to read initial vector row count: {error}");
                0
            }
        };
        Ok(Self {
            index: Some(index),
            initial,
            last: initial,
        })
    }

    /// Re-read the committed row count; a transient read failure keeps the
    /// previous count.
    pub(crate) fn refresh(&mut self) {
        self.last = row_count_or(&self.index, self.last);
    }

    pub(crate) fn available(&self) -> bool {
        self.index.is_some()
    }

    pub(crate) fn total(&self) -> u64 {
        self.last
    }

    pub(crate) fn delta(&self) -> u64 {
        self.last.saturating_sub(self.initial)
    }
}
pub(crate) struct IndexMetrics {
    total: usize,
    started: MonotonicInstant,
    bytes_read: u64,
    last_status: MonotonicInstant,
    projection: ProjectionObserver,
}

impl IndexMetrics {
    pub(crate) fn new(total: usize, layout: &InstanceLayout) -> anyhow::Result<Self> {
        let now = MonotonicInstant::now();
        Ok(Self {
            total,
            started: now,
            bytes_read: 0,
            last_status: now,
            projection: ProjectionObserver::open(layout)?,
        })
    }
    pub(crate) fn add_bytes(&mut self, bytes: u64) {
        self.bytes_read = self.bytes_read.saturating_add(bytes);
    }

    /// Emit a status line at most every five seconds: file progress with
    /// elapsed time and throughput, plus the live embedding row count.
    pub(crate) fn status_line(&mut self, done: usize) -> Option<String> {
        let now = MonotonicInstant::now();
        if now.duration_since(self.last_status) < STATUS_REPORT_INTERVAL {
            return None;
        }
        self.last_status = now;
        self.projection.refresh();
        let elapsed = self.started.elapsed();
        let files_rate = files_per_second(done, elapsed);
        let percent = if self.total == 0 {
            0.0
        } else {
            done as f64 / self.total as f64 * 100.0
        };
        let mut line = format!(
            "status: files {done}/{} ({percent:.1}%) elapsed={} rate={files_rate:.2} \
             files/s bytes={}",
            self.total,
            format_duration(elapsed),
            human_bytes(self.bytes_read),
        );
        if self.projection.available() {
            let delta = self.projection.delta();
            let embed_rate = rate_per_second(delta, elapsed);
            line.push_str(&format!(
                " ; embeddings={} (+{delta}) {embed_rate:.1}/s",
                self.projection.total()
            ));
        }
        Some(line)
    }

    /// Final summary lines: totals, elapsed time, throughput, and the
    /// embedding row count produced by the run.
    pub(crate) fn summary(
        &mut self,
        indexed: usize,
        unchanged: usize,
        skipped: usize,
        failed: usize,
    ) -> String {
        self.projection.refresh();
        let elapsed = self.started.elapsed();
        let processed = indexed + unchanged + skipped + failed;
        let files_rate = files_per_second(processed, elapsed);
        let bytes_per_minute = self.bytes_read as f64 / elapsed.as_secs_f64().max(0.001) / 60.0;
        let mut summary = format!(
            "indexed {indexed} · unchanged {unchanged} · skipped {skipped} · failed {failed}\n\
             duration={} files_rate={files_rate:.2}/s bytes={} bytes_rate=\
             {bytes_per_minute:.2} MiB/min",
            format_duration(elapsed),
            human_bytes(self.bytes_read),
        );
        if self.projection.available() {
            let delta = self.projection.delta();
            let embed_rate = rate_per_second(delta, elapsed);
            summary.push_str(&format!(
                "\nembeddings={} (+{delta} this run) rate={embed_rate:.1}/s",
                self.projection.total()
            ));
        }
        summary
    }
}

fn files_per_second(done: usize, elapsed: Duration) -> f64 {
    done as f64 / elapsed.as_secs_f64().max(0.001)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_duration_with_minutes_and_hours() {
        assert_eq!(format_duration(Duration::from_secs(9)), "0:09");
        assert_eq!(format_duration(Duration::from_secs(65)), "1:05");
        assert_eq!(format_duration(Duration::from_secs(3725)), "1:02:05");
    }

    #[test]
    fn formats_bytes_in_binary_units() {
        assert_eq!(human_bytes(99), "99 B");
        assert_eq!(human_bytes(12 * 1024 + 400), "12.4 KiB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn status_line_emits_at_most_every_five_seconds() -> Result<(), Box<dyn std::error::Error>> {
        let now = MonotonicInstant::now();
        let mut metrics = IndexMetrics {
            total: 10,
            started: now,
            bytes_read: 2048,
            last_status: now.saturating_sub(Duration::from_secs(6)),
            projection: ProjectionObserver {
                index: None,
                initial: 0,
                last: 0,
            },
        };
        let line = match metrics.status_line(4) {
            Some(line) => line,
            None => return Err("status line after the report interval was not emitted".into()),
        };
        assert!(line.contains("files 4/10 (40.0%)"));
        assert!(line.contains("bytes=2.0 KiB"));
        assert!(
            metrics.status_line(5).is_none(),
            "status must wait for the next report interval"
        );
        Ok(())
    }
}
