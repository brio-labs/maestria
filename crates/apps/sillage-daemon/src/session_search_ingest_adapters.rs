use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow};
use sillage_domain::{DEFAULT_CORPUS_SNAPSHOT_ID, DEFAULT_INSTANCE_SCOPE_ID, IndexGenerationId};
use sillage_ports::{
    EvidenceRepository, InMemoryArtifactRepository, InMemoryBlobStore, InMemoryCardRepository,
    InMemoryChunkRepository, InMemoryEvidenceRepository,
};
use sillage_retrieval::{
    HybridExecutionPolicy, LearnedSparseExecutionPolicy, RepositoryExecutionPolicy,
};
use sillage_search_tantivy::{RamMemoryBudget, RamMemoryReservation, TantivyFullTextIndex};
use tokio_util::sync::CancellationToken;

use super::verification::VerificationParser;
use crate::search_executor::SearchRuntimeParts;

pub(in crate::session_search) struct EvidenceOpenContext<'a> {
    pub(in crate::session_search) evidence_id: sillage_domain::EvidenceId,
    pub(in crate::session_search) authorization:
        &'a sillage_governance::RetrievalAuthorizationContext,
    pub(in crate::session_search) cancellation: &'a CancellationToken,
    pub(in crate::session_search) deadline: super::SessionDeadline,
    pub(in crate::session_search) max_file_bytes: u64,
    pub(in crate::session_search) max_work_units: u64,
    pub(in crate::session_search) verification_work: &'a Mutex<u64>,
}

pub(in crate::session_search) struct SessionStores {
    pub(in crate::session_search) artifacts: InMemoryArtifactRepository,
    pub(in crate::session_search) cards: InMemoryCardRepository,
    pub(in crate::session_search) chunks: InMemoryChunkRepository,
    pub(in crate::session_search) evidence: InMemoryEvidenceRepository,
    pub(in crate::session_search) event_log: Arc<sillage_storage_sqlite::SqliteStore>,
    pub(in crate::session_search) blobs: InMemoryBlobStore,
    pub(in crate::session_search) search_index: Arc<TantivyFullTextIndex>,
    pub(in crate::session_search) parser: sillage_parsers::ParserRegistry,
    truncated_excerpts: Mutex<BTreeSet<sillage_domain::EvidenceId>>,
    pub(in crate::session_search) memory: Arc<RamMemoryBudget>,
    retained: Mutex<Vec<RamMemoryReservation>>,
}

impl SessionStores {
    pub(in crate::session_search) fn new(memory: Arc<RamMemoryBudget>) -> Result<Self> {
        let search_index = Arc::new(TantivyFullTextIndex::in_memory_with_budget(
            memory.clone(),
            15_000_000,
        )?);
        let mut parser = sillage_parsers::ParserRegistry::new();
        parser.register(sillage_parsers::MarkdownParser::new());
        Ok(Self {
            artifacts: InMemoryArtifactRepository::new(),
            cards: InMemoryCardRepository::new(),
            chunks: InMemoryChunkRepository::new(),
            evidence: InMemoryEvidenceRepository::new(),
            event_log: Arc::new(sillage_storage_sqlite::SqliteStore::in_memory()?),
            blobs: InMemoryBlobStore::new(),
            search_index,
            parser,
            truncated_excerpts: Mutex::new(BTreeSet::new()),
            memory,
            retained: Mutex::new(Vec::new()),
        })
    }

    pub(in crate::session_search) fn retain(
        &self,
        reservation: RamMemoryReservation,
    ) -> Result<()> {
        self.retained
            .lock()
            .map_err(|error| anyhow!("session admission state lock failed: {error}"))?
            .push(reservation);
        Ok(())
    }

    pub(in crate::session_search) fn retained_bytes(&self) -> Result<u64> {
        Ok(self
            .retained
            .lock()
            .map_err(|error| anyhow!("session admission state lock failed: {error}"))?
            .iter()
            .map(RamMemoryReservation::reserved_bytes)
            .sum())
    }

    pub(in crate::session_search) fn mark_excerpt_truncated(
        &self,
        evidence_id: sillage_domain::EvidenceId,
    ) -> Result<()> {
        self.truncated_excerpts
            .lock()
            .map_err(|error| anyhow!("session excerpt state lock failed: {error}"))?
            .insert(evidence_id);
        Ok(())
    }

    pub(in crate::session_search) fn excerpt_was_truncated(
        &self,
        evidence_id: sillage_domain::EvidenceId,
    ) -> Result<bool> {
        Ok(self
            .truncated_excerpts
            .lock()
            .map_err(|error| anyhow!("session excerpt state lock failed: {error}"))?
            .contains(&evidence_id))
    }

    pub(in crate::session_search) fn runtime_parts(&self) -> SearchRuntimeParts {
        SearchRuntimeParts {
            artifacts: Arc::new(self.artifacts.clone()),
            cards: Arc::new(self.cards.clone()),
            chunks: Arc::new(self.chunks.clone()),
            evidence: Arc::new(self.evidence.clone()),
            search_index: self.search_index.clone(),
            blobs: Arc::new(self.blobs.clone()),
            vector_index: None,
            graph_index: None,
            event_log: self.event_log.clone(),
            primary_generation: IndexGenerationId::new(1),
            dense_generation: None,
            repository_code_index: None,
            repository_execution_policy: RepositoryExecutionPolicy::Shadow,
            hybrid_execution_policy: HybridExecutionPolicy::Shadow,
            learned_sparse_execution_policy: LearnedSparseExecutionPolicy::Disabled,
            sparse_retriever: None,
            corpus_snapshot: DEFAULT_CORPUS_SNAPSHOT_ID,
            scope_id: DEFAULT_INSTANCE_SCOPE_ID,
        }
    }

    pub(in crate::session_search) fn evidence(
        &self,
        evidence_id: sillage_domain::EvidenceId,
    ) -> Result<Option<sillage_domain::Evidence>> {
        Ok(self.evidence.get(evidence_id)?)
    }

    pub(in crate::session_search) fn open_evidence(
        &self,
        context: EvidenceOpenContext<'_>,
    ) -> Result<sillage_core::OpenEvidenceOutput> {
        let EvidenceOpenContext {
            evidence_id,
            authorization,
            cancellation,
            deadline,
            max_file_bytes,
            max_work_units,
            verification_work,
        } = context;
        let parser = VerificationParser::new(
            &self.parser,
            &self.memory,
            cancellation,
            deadline,
            max_file_bytes,
            max_work_units,
            verification_work,
        );
        let core = sillage_core::CoreServices::new(sillage_core::CorePorts {
            artifacts: &self.artifacts,
            chunks: &self.chunks,
            cards: &self.cards,
            evidence: &self.evidence,
            events: self.event_log.as_ref(),
            parser: &parser,
            search_index: self.search_index.as_ref(),
            blobs: &self.blobs,
            vector_index: None,
            graph_index: None,
        });
        let output = core
            .open_evidence_pre_authorized(
                sillage_core::OpenEvidenceInput { evidence_id },
                authorization,
            )
            .map_err(anyhow::Error::new)
            .context("session evidence could not be reopened under current rights");
        parser.check()?;
        output
    }
}
