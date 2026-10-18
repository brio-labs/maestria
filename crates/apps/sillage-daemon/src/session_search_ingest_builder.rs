use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::session_search::deadline::SessionDeadline;
use anyhow::{Result, anyhow, bail};
use sillage_core::build_artifact_detected_input;
use sillage_domain::{
    BlobId, ContentHash, DomainEvent, DomainEventEnvelope, EventId, SourceChoice, content_hash,
};
use sillage_ports::EventLog;
use sillage_search_tantivy::{MemoryBudgetError, RamMemoryReservation};
use tokio_util::sync::CancellationToken;

use super::{SessionCoverage, SessionStores, ensure_not_cancelled, scan, store};
use crate::session_search::{
    native::AcquiredFile,
    retained::{
        OwnershipPermit, PersistedGeneration, RetainedDocumentaryStore, RetainedSourceObject,
        RetainedSources,
    },
};

#[path = "session_search_ingest_parse.rs"]
mod parse;
#[cfg(test)]
pub(in crate::session_search) use parse::{
    PARSE_TEST_SERIAL, ParseCheckpointGate, clear_gate, install_gate,
};

pub(super) struct SessionBuilder<'a> {
    choice: &'a SourceChoice,
    stores: Arc<SessionStores>,
    selected_path: PathBuf,
    documentary: Option<&'a RetainedDocumentaryStore>,
    deadline: SessionDeadline,
    next_event_id: u64,
    read_admission: Option<RamMemoryReservation>,
    pub(super) runtime_admission: Vec<RamMemoryReservation>,
    pub(super) retained_sources: RetainedSources,
    pub(super) retained_generation: Option<PersistedGeneration>,
    work_units: u64,
    pub(super) coverage: SessionCoverage,
}

enum SourceRead {
    Skip,
    Stop,
    Bytes {
        bytes: Vec<u8>,
        source: Arc<RetainedSourceObject>,
    },
}

impl<'a> SessionBuilder<'a> {
    pub(super) fn new(
        choice: &'a SourceChoice,
        stores: Arc<SessionStores>,
        selected_path: PathBuf,
        documentary: Option<&'a RetainedDocumentaryStore>,
        deadline: SessionDeadline,
    ) -> Self {
        Self {
            choice,
            stores,
            selected_path,
            documentary,
            deadline,
            next_event_id: 1,
            read_admission: None,
            runtime_admission: Vec::new(),
            retained_sources: RetainedSources::new(),
            retained_generation: None,
            work_units: 0,
            coverage: SessionCoverage::new(),
        }
    }

    pub(super) fn deadline(&self) -> SessionDeadline {
        self.deadline
    }

    pub(super) fn memory(&self) -> Arc<sillage_search_tantivy::RamMemoryBudget> {
        self.stores.memory.clone()
    }

    pub(super) fn visit_file(
        &mut self,
        candidate: &Path,
        acquired: AcquiredFile<OwnershipPermit>,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        self.read_admission = None;
        ensure_not_cancelled(cancellation)?;
        if self.deadline_reached() {
            self.defer();
            return Ok(());
        }
        if sillage_index_selection::is_privacy_excluded_path(candidate) {
            self.exclude_source();
            bail!("selected source is excluded by the local privacy policy");
        }
        if candidate != self.selected_path
            || !scan::scope_contains(self.choice.scope(), &self.selected_path, candidate)
        {
            self.fail_source();
            bail!("selected source escaped its explicit file scope");
        }
        if !matches!(
            candidate.extension().and_then(|value| value.to_str()),
            Some(extension) if extension.eq_ignore_ascii_case("md")
                || extension.eq_ignore_ascii_case("markdown")
        ) {
            self.unsupported();
            return Ok(());
        }
        let (bytes, source) = match self.read_source(acquired, cancellation)? {
            SourceRead::Skip => return Ok(()),
            SourceRead::Stop => return Ok(()),
            SourceRead::Bytes { bytes, source } => (bytes, source),
        };
        let hash = ContentHash::new(content_hash(&bytes))?;
        let input = match build_artifact_detected_input(candidate, bytes, hash.as_str().to_owned())?
        {
            sillage_domain::DomainInput::ArtifactDetected(input) => input,
            _ => bail!("selected source did not produce artifact detection input"),
        };
        let deferred_before = self.coverage.deferred_entries;
        let Some((parsed, _parse_admission)) =
            self.parse_source(candidate, &input, cancellation)?
        else {
            if self.coverage.deferred_entries > deferred_before {
                return Ok(());
            }
            return Ok(());
        };
        ensure_not_cancelled(cancellation)?;
        if self.deadline_reached() {
            self.defer();
            return Ok(());
        }
        if parsed.artifact_id != input.artifact_id || parsed.content_hash != input.content_hash {
            self.fail_source();
            bail!("selected Markdown parser changed the source identity");
        }
        self.store_source(candidate, input, &parsed, source)
    }

    fn read_source(
        &mut self,
        acquired: AcquiredFile<OwnershipPermit>,
        cancellation: &CancellationToken,
    ) -> Result<SourceRead> {
        ensure_not_cancelled(cancellation)?;
        if self.deadline_reached() {
            self.defer();
            return Ok(SourceRead::Stop);
        }
        let file_size = acquired.length();
        let budget = self.choice.budget();
        if file_size > budget.max_file_bytes() {
            self.coverage.oversized_sources = self.coverage.oversized_sources.saturating_add(1);
            self.coverage.coverage_complete = false;
            return Ok(SourceRead::Skip);
        }
        if self.coverage.bytes_examined.saturating_add(file_size) > budget.max_total_source_bytes()
            || self.work_units.saturating_add(file_size) > budget.max_work_units()
        {
            self.defer();
            return Ok(SourceRead::Stop);
        }
        let read_admission = self
            .stores
            .memory
            .reserve(file_size.saturating_add(16_385))?;
        let deadline = self.deadline();
        let mut examined = 0_u64;
        let bytes = scan::read_source_bounded(
            acquired.file(),
            file_size,
            cancellation,
            deadline,
            |count| {
                examined = examined
                    .checked_add(count)
                    .ok_or_else(super::verification::work_budget_exceeded)?;
                if self.coverage.bytes_examined.saturating_add(examined)
                    > budget.max_total_source_bytes()
                    || self.work_units.saturating_add(examined) > budget.max_work_units()
                {
                    return Err(super::verification::work_budget_exceeded());
                }
                Ok(())
            },
        );
        self.coverage.bytes_examined = self.coverage.bytes_examined.saturating_add(examined);
        self.work_units = self.work_units.saturating_add(examined);
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(error) if cancellation.is_cancelled() => return Err(error),
            Err(error)
                if error
                    .downcast_ref::<super::VerificationBudgetExceeded>()
                    .is_some() =>
            {
                self.defer();
                return Ok(SourceRead::Stop);
            }
            Err(error)
                if crate::SessionOperationSettlement::classify_error(&error)
                    == Some(crate::SessionOperationSettlement::DeadlineExceeded) =>
            {
                self.defer();
                return Ok(SourceRead::Stop);
            }
            Err(error) => return Err(error),
        };
        ensure_not_cancelled(cancellation)?;
        if self.deadline_reached() {
            self.defer();
            return Ok(SourceRead::Stop);
        }
        if u64::try_from(bytes.len())? != file_size {
            self.fail_source();
            bail!("selected Markdown source changed while it was being read");
        }
        self.read_admission = Some(read_admission);
        Ok(SourceRead::Bytes {
            bytes,
            source: RetainedSourceObject::from_acquired(acquired),
        })
    }

    fn store_source(
        &mut self,
        path: &Path,
        input: sillage_domain::ArtifactDetected,
        parsed: &sillage_ports::ParsedArtifact,
        source: Arc<RetainedSourceObject>,
    ) -> Result<()> {
        let binding_admission = match self.retained_sources.admit_binding(
            parsed.artifact_id,
            parsed.artifact_version_id,
            path,
            &self.stores.memory,
        ) {
            Ok(admission) => admission,
            Err(error)
                if error
                    .downcast_ref::<MemoryBudgetError>()
                    .is_some_and(|error| matches!(error, MemoryBudgetError::Exhausted { .. })) =>
            {
                self.defer();
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let parsed_bytes = store::parsed_memory_estimate(parsed, path.as_os_str().len())?;
        let retained = match self.stores.memory.reserve(parsed_bytes) {
            Ok(reservation) => reservation,
            Err(MemoryBudgetError::Exhausted { .. }) => {
                self.defer();
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let projection = match self.stores.memory.reserve(parsed_bytes.saturating_mul(3)) {
            Ok(reservation) => reservation,
            Err(MemoryBudgetError::Exhausted { .. }) => {
                self.defer();
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        if self.work_units.saturating_add(parsed_bytes) > self.choice.budget().max_work_units() {
            self.defer();
            return Ok(());
        }
        let title = input.title;
        let mut source_admission = self
            .read_admission
            .take()
            .ok_or_else(|| anyhow!("selected source read admission is missing"))?;
        source_admission.resize(u64::try_from(input.source_bytes.capacity())?)?;
        let stored = store::store_parsed_source(
            &self.stores,
            &title,
            path,
            parsed,
            input.source_bytes,
            self.documentary,
        )?;
        let (blob_id, generation) = match stored {
            store::ParsedSourceStoreOutcome::Stored {
                blob_id,
                generation,
            } => (blob_id, generation),
            store::ParsedSourceStoreOutcome::SecretRejected => {
                self.fail_source();
                return Ok(());
            }
        };
        self.retained_generation = generation;
        self.append_source_events(&title, path, parsed, blob_id)?;
        self.stores.retain(retained)?;
        self.stores.retain(source_admission)?;
        self.runtime_admission.push(projection);
        self.retained_sources.publish(
            parsed.artifact_id,
            parsed.artifact_version_id,
            path.to_path_buf(),
            parsed.content_hash.clone(),
            source,
            binding_admission,
        )?;
        self.work_units = self.work_units.saturating_add(parsed_bytes);
        self.coverage.indexed_sources = self.coverage.indexed_sources.saturating_add(1);
        Ok(())
    }

    fn append_source_events(
        &mut self,
        title: &str,
        path: &Path,
        parsed: &sillage_ports::ParsedArtifact,
        blob_id: BlobId,
    ) -> Result<()> {
        let source_path = path
            .to_str()
            .ok_or_else(|| anyhow!("source path is not UTF-8"))?
            .to_owned();
        self.append_event(DomainEvent::ParserStarted {
            artifact_id: parsed.artifact_id,
            title: title.to_owned(),
            source_path,
            content_hash: parsed.content_hash.clone(),
            blob_id,
        })?;
        self.append_event(DomainEvent::DocumentTreeCaptured {
            artifact_id: parsed.artifact_id,
            artifact_version_id: parsed.artifact_version_id,
            content_hash: parsed.content_hash.clone(),
            root_id: parsed.tree.root_id(),
            nodes: parsed.tree.nodes().to_vec(),
        })
    }

    fn append_event(&mut self, event: DomainEvent) -> Result<()> {
        self.stores.event_log.append(DomainEventEnvelope {
            id: EventId::new(self.next_event_id),
            event,
        })?;
        self.next_event_id = self
            .next_event_id
            .checked_add(1)
            .ok_or_else(|| anyhow!("session source event identity overflow"))?;
        Ok(())
    }

    pub(super) fn deadline_reached(&self) -> bool {
        self.deadline.is_elapsed()
    }

    pub(super) fn defer(&mut self) {
        self.coverage.deferred_entries = self.coverage.deferred_entries.saturating_add(1);
        self.coverage.coverage_complete = false;
    }

    pub(super) fn exclude_source(&mut self) {
        self.coverage.excluded_sources = self.coverage.excluded_sources.saturating_add(1);
        self.coverage.coverage_complete = false;
    }

    pub(super) fn fail_source(&mut self) {
        self.coverage.failed_sources = self.coverage.failed_sources.saturating_add(1);
        self.coverage.coverage_complete = false;
    }

    fn unsupported(&mut self) {
        self.coverage.unsupported_sources = self.coverage.unsupported_sources.saturating_add(1);
        self.coverage.coverage_complete = false;
    }
}

#[cfg(all(test, unix))]
#[path = "session_search_ingest_builder_tests.rs"]
mod tests;
