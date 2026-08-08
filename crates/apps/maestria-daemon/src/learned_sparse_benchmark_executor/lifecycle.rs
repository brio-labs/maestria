//! Lifecycle operations measured on the route's projection.

use std::path::Path;

use anyhow::{Result, anyhow};
use maestria_domain::{
    Chunk, ChunkId, ContentHash, IndexLifecycle, TransitionIndexGenerationInput,
};
use maestria_ports::{
    IndexedChunk, LearnedSparseIndex, LearnedSparseProjectionLifecycle, LearnedSparseProvider,
    SparseDocument, SparseIdentity, SparseInputKind,
};
use maestria_retrieval::{
    LearnedSparseOperationMeasurement, LearnedSparseRoute, Measurement, MonotonicInstant,
};

use super::LearnedSparseBenchmarkExecutor;
use super::energy::EnergySample;

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
            LearnedSparseRoute::Lexical | LearnedSparseRoute::Hybrid => {
                Self::dir_size(&self.layout.full_text_index_dir)
            }
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
        reason: String,
    ) -> LearnedSparseOperationMeasurement {
        LearnedSparseOperationMeasurement {
            elapsed_ms: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
            throughput_items_per_second: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
            cost_micros: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
            energy_millijoules: Measurement::unavailable(format!(
                "{operation} on the {route:?} projection: {reason}"
            )),
        }
    }

    /// Converts a timed operation into the typed measurement.
    fn finish_measurement(
        route: LearnedSparseRoute,
        operation: &str,
        items: usize,
        started: MonotonicInstant,
        energy_before: Option<EnergySample>,
        result: Result<(), anyhow::Error>,
    ) -> LearnedSparseOperationMeasurement {
        let elapsed = started.elapsed();
        let elapsed_us = elapsed.as_micros() as u64;
        let energy_after = EnergySample::capture();
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
                energy_millijoules: EnergySample::delta_uj_pair(energy_before, energy_after),
            },
            Err(error) => {
                let reason = format!("{operation} on the {route:?} projection failed: {error}");
                LearnedSparseOperationMeasurement {
                    elapsed_ms: Measurement::unavailable(reason.clone()),
                    throughput_items_per_second: Measurement::unavailable(reason.clone()),
                    cost_micros: Measurement::unavailable(reason.clone()),
                    energy_millijoules: EnergySample::delta_uj_pair(energy_before, energy_after),
                }
            }
        }
    }

    /// Measures one lifecycle operation on the route's projection.
    fn measure_operation(
        &self,
        route: LearnedSparseRoute,
        operation: &str,
        items: usize,
        op: impl Fn() -> Result<(), anyhow::Error>,
    ) -> LearnedSparseOperationMeasurement {
        let started = MonotonicInstant::now();
        let energy_before = EnergySample::capture();
        let result = op();
        Self::finish_measurement(route, operation, items, started, energy_before, result)
    }

    fn encode_documents(
        provider: &(dyn LearnedSparseProvider + Send + Sync),
        identity: &SparseIdentity,
        chunks: &[Chunk],
    ) -> Result<Vec<SparseDocument>> {
        let encoded_texts = chunks
            .iter()
            .map(|chunk| crate::sparse_startup::truncate_document_text(&chunk.text))
            .collect::<Vec<_>>();
        let vectors =
            provider.encode_batch(&encoded_texts, SparseInputKind::Document, identity.clone())?;
        chunks
            .iter()
            .zip(vectors)
            .map(|(chunk, vector)| {
                if vector.identity() != identity {
                    return Err(anyhow!(
                        "encode chunk {} returned an incompatible generation identity",
                        chunk.id
                    ));
                }
                let content_hash =
                    ContentHash::new(maestria_domain::content_hash(chunk.text.as_bytes()))?;
                Ok(SparseDocument {
                    chunk_id: chunk.id,
                    content_hash,
                    vector,
                })
            })
            .collect()
    }

    /// The real instance chunks reduced to the full-text projection input.
    fn indexed_chunks(&self) -> Vec<IndexedChunk> {
        self.chunks
            .iter()
            .map(|chunk| IndexedChunk {
                artifact_id: chunk.artifact_id,
                chunk_id: chunk.id,
                text: chunk.text.clone(),
            })
            .collect()
    }

    fn chunk_identities(&self) -> Vec<(maestria_domain::ArtifactId, ChunkId)> {
        self.chunks
            .iter()
            .map(|chunk| (chunk.artifact_id, chunk.id))
            .collect()
    }

    /// Lifecycle operations on the shared full-text projection, which is the
    /// durable projection of the lexical route and of the hybrid route in
    /// this evaluation (no dense provider is configured, per the plan's C1
    /// scope). Activation and rollback are the real registry transitions on
    /// the lexical generation, restored to Active before the operation ends.
    fn lexical_lifecycle_operations(
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
        let index = self.runtime.search_index.clone();
        let chunks = self.indexed_chunks();
        let identities = self.chunk_identities();
        let one = match identities.first() {
            Some(identity) => *identity,
            None => return self.empty_chunk_ops(route),
        };
        let initial = self.measure_operation(route, "initial indexing", chunks.len(), || {
            index
                .index_chunks(chunks.clone())
                .map_err(anyhow::Error::from)
        });
        let incremental = self.measure_operation(route, "incremental update", 1, || {
            let one_chunk = chunks
                .iter()
                .filter(|chunk| chunk.artifact_id == one.0 && chunk.chunk_id == one.1)
                .cloned()
                .collect::<Vec<_>>();
            index.index_chunks(one_chunk).map_err(anyhow::Error::from)
        });
        let deletion = self.measure_operation(route, "deletion", 1, || {
            index.delete_chunks(&[one]).map_err(anyhow::Error::from)
        });
        let rebuild = self.measure_operation(route, "rebuild", chunks.len(), || {
            index.clear().map_err(anyhow::Error::from)?;
            index
                .index_chunks(chunks.clone())
                .map_err(anyhow::Error::from)
        });
        let (activation, rollback) = self.generation_transition_ops(route);
        (
            initial,
            incremental,
            deletion,
            rebuild,
            activation,
            rollback,
        )
    }

    /// Activation and rollback measured as the real registry transitions on
    /// the route's generation, each restored before the pair completes.
    ///
    /// Both transitions run against one state clone so the second transition
    /// validates against the first transition's outcome.
    fn generation_transition_ops(
        &self,
        route: LearnedSparseRoute,
    ) -> (
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
    ) {
        let Some(generation_id) = self
            .state
            .index_generations
            .get_active(&maestria_domain::RepresentationName::new("lexical_text_v1"))
            .map(|generation| generation.id)
        else {
            let reason = "the lexical generation is not active in this instance".to_string();
            return (
                Self::unavailable_operation(route, "activation", reason.clone()),
                Self::unavailable_operation(route, "rollback", reason),
            );
        };
        // Rollback first: the evaluated generation is active, so the measured
        // rollback is the real Active -> Retired transition and the measured
        // activation is the Retired -> Active path the registry rollback uses.
        let mut state = self.state.clone();
        let transition = |state: &mut maestria_domain::KernelState, to: IndexLifecycle| {
            crate::vector_startup::persist_input(
                state,
                &self.store,
                maestria_domain::DomainInput::TransitionIndexGeneration(
                    TransitionIndexGenerationInput {
                        id: generation_id,
                        to,
                    },
                ),
            )
        };
        let rollback_started = MonotonicInstant::now();
        let rollback_energy = EnergySample::capture();
        let rollback_result = transition(&mut state, IndexLifecycle::Retired);
        let rollback = Self::finish_measurement(
            route,
            "rollback",
            0,
            rollback_started,
            rollback_energy,
            rollback_result,
        );
        let activation_started = MonotonicInstant::now();
        let activation_energy = EnergySample::capture();
        let activation_result = transition(&mut state, IndexLifecycle::Active);
        let activation = Self::finish_measurement(
            route,
            "activation",
            0,
            activation_started,
            activation_energy,
            activation_result,
        );
        (activation, rollback)
    }

    fn empty_chunk_ops(
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
        let reason = "the evaluated instance has no indexed chunks".to_string();
        (
            Self::unavailable_operation(route, "initial indexing", reason.clone()),
            Self::unavailable_operation(route, "incremental update", reason.clone()),
            Self::unavailable_operation(route, "deletion", reason.clone()),
            Self::unavailable_operation(route, "rebuild", reason.clone()),
            Self::unavailable_operation(route, "activation", reason.clone()),
            Self::unavailable_operation(route, "rollback", reason),
        )
    }

    /// Lifecycle operations for one route, measured on its projection.
    ///
    /// The sparse projection's evaluated lifecycle is already active, so the
    /// measured rollback is the real Active -> Retired transition and the
    /// measured activation is the Retired -> Active path the registry
    /// rollback uses.
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
        match (route, &self.sparse) {
            (LearnedSparseRoute::Lexical | LearnedSparseRoute::Hybrid, _) => {
                self.lexical_lifecycle_operations(route)
            }
            (LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused, Some(lane)) => {
                self.sparse_lifecycle_operations(route, lane)
            }
            (LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused, None) => {
                self.empty_chunk_ops(route)
            }
        }
    }

    fn sparse_lifecycle_operations(
        &self,
        route: LearnedSparseRoute,
        lane: &super::SparseLane,
    ) -> (
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
        LearnedSparseOperationMeasurement,
    ) {
        let index = lane.index.clone();
        let identity = lane.identity.clone();
        let provider = lane.provider.clone();
        let chunks = self.chunks.clone();
        let one_chunk = match chunks.first() {
            Some(chunk) => chunk.id,
            None => return self.empty_chunk_ops(route),
        };
        let initial = self.measure_operation(route, "initial indexing", chunks.len(), || {
            let documents = Self::encode_documents(provider.as_ref(), &identity, &chunks)?;
            index
                .index_documents(documents)
                .map_err(anyhow::Error::from)
        });
        let incremental = self.measure_operation(route, "incremental update", 1, || {
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
        let deletion = self.measure_operation(route, "deletion", 1, || {
            index
                .delete_chunks(&[one_chunk])
                .map_err(anyhow::Error::from)
        });
        let rebuild = self.measure_operation(route, "rebuild", chunks.len(), || {
            let documents = Self::encode_documents(provider.as_ref(), &identity, &chunks)?;
            index.rebuild(documents).map_err(anyhow::Error::from)
        });
        let rollback = self.measure_operation(route, "rollback", 0, || {
            index
                .transition(
                    maestria_domain::IndexLifecycle::Active,
                    maestria_domain::IndexLifecycle::Retired,
                )
                .map_err(anyhow::Error::from)
        });
        let activation = self.measure_operation(route, "activation", 0, || {
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
