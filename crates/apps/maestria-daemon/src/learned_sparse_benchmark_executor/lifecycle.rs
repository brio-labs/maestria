//! Lifecycle operations measured on the route's projection.

use std::path::Path;

use anyhow::{Result, anyhow};
use maestria_domain::{Chunk, ChunkId, ContentHash};
use maestria_ports::{
    LearnedSparseIndex, LearnedSparseProjectionLifecycle, LearnedSparseProvider, SparseDocument,
    SparseIdentity, SparseInputKind,
};
use maestria_retrieval::{
    LearnedSparseOperationMeasurement, LearnedSparseRoute, Measurement, MonotonicInstant,
};

use super::LearnedSparseBenchmarkExecutor;

impl LearnedSparseBenchmarkExecutor {
    /// Peak resident set size of this process, in bytes.
    pub(super) fn peak_ram_bytes(&self) -> u64 {
        let status = match std::fs::read_to_string("/proc/self/status") {
            Ok(status) => status,
            Err(_) => return 0,
        };
        let kilobytes = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|value| value.trim().strip_suffix(" kB"))
            .and_then(|value| value.trim().parse::<u64>().ok());
        match kilobytes {
            Some(kb) => kb.saturating_mul(1024),
            None => 0,
        }
    }

    fn dir_size(path: &Path) -> u64 {
        fn walk(path: &Path, total: &mut u64) {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        walk(&entry_path, total);
                    } else if let Ok(metadata) = entry.metadata() {
                        *total = total.saturating_add(metadata.len());
                    }
                }
            }
        }
        let mut total = 0_u64;
        walk(path, &mut total);
        total
    }

    /// Index footprint of the route's projection, in bytes.
    pub(super) fn index_disk_bytes(&self, route: LearnedSparseRoute) -> u64 {
        match route {
            LearnedSparseRoute::Lexical => Self::dir_size(&self.layout.full_text_index_dir),
            LearnedSparseRoute::Hybrid => Self::dir_size(&self.layout.vector_index_dir),
            LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused => {
                match std::fs::metadata(&self.layout.database_path) {
                    Ok(metadata) => metadata.len(),
                    Err(_) => 0,
                }
            }
        }
    }

    fn unavailable_operation(
        route: LearnedSparseRoute,
        operation: &str,
    ) -> LearnedSparseOperationMeasurement {
        LearnedSparseOperationMeasurement {
            elapsed_ms: Measurement::unavailable(format!(
                "{operation} is not exposed as a standalone operation on the {route:?} projection"
            )),
            throughput_items_per_second: Measurement::unavailable(format!(
                "{operation} is not exposed as a standalone operation on the {route:?} projection"
            )),
            cost_micros: Measurement::unavailable(format!(
                "{operation} is not exposed as a standalone operation on the {route:?} projection"
            )),
            energy_millijoules: Measurement::unavailable(
                "RAPL energy_uj is not readable without privileges on this host",
            ),
        }
    }

    /// Measures one lifecycle operation on the route's projection.
    fn measure_operation(
        &self,
        _route: LearnedSparseRoute,
        items: usize,
        op: impl Fn() -> Result<(), anyhow::Error>,
    ) -> LearnedSparseOperationMeasurement {
        let started = MonotonicInstant::now();
        let result = op();
        let elapsed = started.elapsed();
        let elapsed_us = elapsed.as_micros() as u64;
        let energy = Measurement::unavailable(
            "RAPL energy_uj is not readable without privileges on this host",
        );
        match result {
            Ok(()) => LearnedSparseOperationMeasurement {
                elapsed_ms: Measurement::measured(elapsed_us.saturating_div(1_000)),
                throughput_items_per_second: Measurement::measured(
                    match (items as u128)
                        .saturating_mul(1_000_000)
                        .checked_div(elapsed.as_micros().max(1))
                    {
                        Some(value) => value as u64,
                        None => 0,
                    },
                ),
                cost_micros: Measurement::measured(elapsed_us),
                energy_millijoules: energy,
            },
            Err(error) => LearnedSparseOperationMeasurement {
                elapsed_ms: Measurement::unavailable(format!(
                    "lifecycle operation failed: {error}"
                )),
                throughput_items_per_second: Measurement::unavailable(
                    "lifecycle operation failed".to_string(),
                ),
                cost_micros: Measurement::unavailable("lifecycle operation failed".to_string()),
                energy_millijoules: energy,
            },
        }
    }

    fn encode_documents(
        provider: &(dyn LearnedSparseProvider + Send + Sync),
        identity: &SparseIdentity,
        chunks: &[Chunk],
    ) -> Result<Vec<SparseDocument>> {
        chunks
            .iter()
            .map(|chunk| {
                let content_hash =
                    ContentHash::new(maestria_domain::content_hash(chunk.text.as_bytes()))?;
                let encoded_text = crate::sparse_startup::truncate_document_text(&chunk.text);
                let vector =
                    provider.encode(&encoded_text, SparseInputKind::Document, identity.clone())?;
                if vector.identity() != identity {
                    return Err(anyhow!(
                        "encode chunk {} returned an incompatible generation identity",
                        chunk.id
                    ));
                }
                Ok(SparseDocument {
                    chunk_id: chunk.id,
                    content_hash,
                    vector,
                })
            })
            .collect()
    }

    /// Lifecycle operations for one route, measured on its projection.
    ///
    /// The evaluated projection is already active, so the measured rollback
    /// is the real Active -> Retired transition and the measured activation
    /// is the Retired -> Active path the registry rollback uses.
    pub(super) fn lifecycle_operations(
        &self,
        route: LearnedSparseRoute,
    ) -> (
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
    ) {
        let Some(lane) = &self.sparse else {
            return (
                Self::unavailable_operation(route, "initial indexing"),
                Self::unavailable_operation(route, "incremental update"),
                Self::unavailable_operation(route, "deletion"),
                Self::unavailable_operation(route, "rebuild"),
                Self::unavailable_operation(route, "activation"),
                Self::unavailable_operation(route, "rollback"),
            );
        };
        let index = lane.index.clone();
        let identity = lane.identity.clone();
        let provider = lane.provider.clone();
        let chunks = self.chunks.clone();
        let one_chunk = match chunks.first() {
            Some(chunk) => chunk.id,
            None => ChunkId::new(0),
        };
        let initial = self.measure_operation(route, chunks.len(), || {
            let documents = Self::encode_documents(provider.as_ref(), &identity, &chunks)?;
            index
                .index_documents(documents)
                .map_err(anyhow::Error::from)
        });
        let incremental = self.measure_operation(route, 1, || {
            let one = chunks
                .iter()
                .filter(|chunk| chunk.id == one_chunk)
                .cloned()
                .collect::<Vec<_>>();
            let documents = Self::encode_documents(provider.as_ref(), &identity, &one)?;
            index
                .index_documents(documents)
                .map_err(anyhow::Error::from)
        });
        let deletion = self.measure_operation(route, 1, || {
            index
                .delete_chunks(&[one_chunk])
                .map_err(anyhow::Error::from)
        });
        let rebuild = self.measure_operation(route, chunks.len(), || {
            let documents = Self::encode_documents(provider.as_ref(), &identity, &chunks)?;
            index.rebuild(documents).map_err(anyhow::Error::from)
        });
        let rollback = self.measure_operation(route, 0, || {
            index
                .transition(
                    maestria_domain::IndexLifecycle::Active,
                    maestria_domain::IndexLifecycle::Retired,
                )
                .map_err(anyhow::Error::from)
        });
        let activation = self.measure_operation(route, 0, || {
            index
                .transition(
                    maestria_domain::IndexLifecycle::Retired,
                    maestria_domain::IndexLifecycle::Active,
                )
                .map_err(anyhow::Error::from)
        });
        (
            initial,
            incremental,
            deletion,
            rebuild,
            activation,
            rollback,
        )
    }
}
