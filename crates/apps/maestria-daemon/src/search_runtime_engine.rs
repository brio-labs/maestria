use std::collections::BTreeSet;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use maestria_domain::content_hash;
use maestria_retrieval::adapters::{
    CardRetriever, CardRetrieverParts, CodeIntelRetriever, CodeIntelRetrieverParts,
    CodeIntelSecurityResolver, CodeIntelSecurityResolverParts, CurrentVersionFilter,
    DenseChunkRetriever, DenseChunkRetrieverParts, EvidenceOutcomeEvaluator,
    HierarchyGraphExpander, HierarchyGraphExpanderParts, LexicalChunkRetriever,
    LexicalChunkRetrieverParts,
};
use maestria_retrieval::{
    CandidateRetriever, FixedKRrf, HybridExecutionPolicy, LateInteractionReranker, RetrievalEngine,
};

use super::{SearchRuntime, reconcile_active_versions};

fn parse_active_late_interaction_evidence(
    promotion_hash: &str,
    promotion_json: &str,
    stage_a_hash: &str,
    stage_a_json: &str,
) -> Option<(
    maestria_retrieval::LateInteractionStageAPromotionRecord,
    maestria_retrieval::LateInteractionStageAReport,
)> {
    if promotion_hash != content_hash(promotion_json.as_bytes())
        || stage_a_hash != content_hash(stage_a_json.as_bytes())
    {
        tracing::warn!("late interaction promotion/report hash mismatch; serving baseline");
        return None;
    }
    let record = serde_json::from_str(promotion_json).ok()?;
    let report = serde_json::from_str(stage_a_json).ok()?;
    Some((record, report))
}

impl SearchRuntime {
    /// The production engine: the loaded hybrid policy, sparse policy, and
    /// the active late-interaction reranker when a validated class record is
    /// present.
    pub(crate) fn retrieval_engine(&self) -> Result<RetrievalEngine> {
        self.refresh_late_interaction_activation()?;
        let mut engine = self.retrieval_engine_with_policies(
            self.hybrid_execution_policy.clone(),
            self.learned_sparse_execution_policy.clone(),
            self.sparse_retriever.clone(),
            true,
        )?;
        if let Some(reranker) = self.late_interaction_active_reranker.clone() {
            engine = engine.with_late_interaction_reranker(reranker);
        }
        Ok(engine)
    }

    /// One shared assembly for every engine variant.
    ///
    /// The benchmark executor and the daemon both build engines here (R28);
    /// only the policies, the optional learned-sparse lane, and whether the
    /// base retrievers are registered differ.
    /// The base serving lanes: cards, lexical chunks, repository code, and dense
    /// chunks, all generation-filtered.
    fn base_retrievers(
        &self,
        events: &[maestria_domain::DomainEventEnvelope],
        sources: &std::collections::BTreeMap<
            std::path::PathBuf,
            (
                maestria_domain::ArtifactId,
                maestria_domain::ArtifactVersionId,
                maestria_domain::ContentHash,
            ),
        >,
        active_versions: std::collections::BTreeSet<maestria_domain::ArtifactVersionId>,
    ) -> Result<Vec<Arc<dyn CandidateRetriever>>> {
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = Vec::new();
        retrievers.push(Arc::new(CurrentVersionFilter::new(
            Arc::new(CardRetriever::new(
                CardRetrieverParts {
                    index: self.search_index.clone(),
                    artifacts: self.artifacts.clone(),
                    cards: self.cards.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.primary_generation,
            )),
            active_versions.clone(),
        )));
        retrievers.push(Arc::new(CurrentVersionFilter::new(
            Arc::new(LexicalChunkRetriever::new(
                LexicalChunkRetrieverParts {
                    index: self.search_index.clone(),
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                self.primary_generation,
            )),
            active_versions.clone(),
        )));
        if let Some(index) = self.repository_code_index.clone() {
            let security = CodeIntelSecurityResolver::from_events(
                CodeIntelSecurityResolverParts {
                    artifacts: self.artifacts.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
                sources,
                events,
            )
            .map_err(|error| anyhow!("prepare repository code security resolver: {error}"))?;
            retrievers.push(Arc::new(CodeIntelRetriever::new(
                CodeIntelRetrieverParts { index, security },
                self.primary_generation,
            )));
        }
        if let (Some(vector_index), Some(provider), Some(generation)) = (
            self.vector_index.clone(),
            self.embedding_provider.clone(),
            self.dense_generation,
        ) {
            retrievers.push(Arc::new(CurrentVersionFilter::new(
                Arc::new(DenseChunkRetriever::new(
                    DenseChunkRetrieverParts {
                        index: vector_index,
                        artifacts: self.artifacts.clone(),
                        chunks: self.chunks.clone(),
                        evidence: self.evidence.clone(),
                        blobs: self.blobs.clone(),
                        embedding_provider: provider,
                    },
                    generation,
                )),
                active_versions.clone(),
            )));
        }
        Ok(retrievers)
    }

    pub(crate) fn retrieval_engine_with_policies(
        &self,
        hybrid_policy: HybridExecutionPolicy,
        sparse_policy: maestria_retrieval::LearnedSparseExecutionPolicy,
        sparse_retriever: Option<Arc<dyn CandidateRetriever>>,
        include_base_retrievers: bool,
    ) -> Result<RetrievalEngine> {
        let events = self.domain_events()?;
        // Single projection scan shared by the version filter and the
        // repository-code security resolver.
        let sources = maestria_domain::active_source_versions(&events);
        let active_versions = reconcile_active_versions(&sources);
        let mut retrievers: Vec<Arc<dyn CandidateRetriever>> = Vec::new();
        if include_base_retrievers {
            retrievers = self.base_retrievers(&events, &sources, active_versions)?;
        }
        // The sparse lane registers after the base lanes so the engine's
        // primary generation stays the lexical generation (R24).
        if let Some(sparse_retriever) = sparse_retriever {
            retrievers.push(sparse_retriever);
        }
        let mut engine = RetrievalEngine::new(
            retrievers,
            Arc::new(EvidenceOutcomeEvaluator::new(self.evidence.clone())),
            self.retrieval_policy.clone(),
        )
        .with_fusion(Arc::new(FixedKRrf::new(60)));
        if self.persist_learned_sparse_observations {
            engine = engine.with_learned_sparse_observation_repository(self.event_log.clone());
        }
        if let Some(graph) = self.graph_index.clone() {
            engine = engine.with_expander(Arc::new(HierarchyGraphExpander::new(
                HierarchyGraphExpanderParts {
                    graph,
                    artifacts: self.artifacts.clone(),
                    chunks: self.chunks.clone(),
                    evidence: self.evidence.clone(),
                    blobs: self.blobs.clone(),
                },
            )));
        }
        Ok(engine
            .with_hybrid_policy(hybrid_policy)
            .with_learned_sparse_execution_policy(sparse_policy)
            .with_repository_execution_policy(self.repository_execution_policy.clone()))
    }

    pub(crate) fn refresh_late_interaction_activation(&self) -> Result<()> {
        let Some(controller) = self.late_interaction_active_controller.as_ref() else {
            return Ok(());
        };
        let allowed = match self.load_active_late_interaction_classes(controller) {
            Ok(allowed) => allowed,
            Err(error) => {
                tracing::warn!(
                    "late interaction activation evidence is unavailable; serving baseline: {error:#}"
                );
                BTreeSet::new()
            }
        };
        controller
            .set_allowed_intents(Some(allowed))
            .map_err(|error| anyhow!("refresh late interaction activation: {error}"))
    }

    fn load_active_late_interaction_classes(
        &self,
        controller: &LateInteractionReranker,
    ) -> Result<BTreeSet<maestria_domain::SearchIntent>> {
        let empty = BTreeSet::new();
        let Some(promotion) = self
            .event_log
            .load_latest_late_interaction_promotion_record()?
        else {
            return Ok(empty);
        };
        let Some(stage_a) = self
            .event_log
            .load_latest_late_interaction_report("stage-a")?
        else {
            return Ok(empty);
        };
        let Some((record, report)) = parse_active_late_interaction_evidence(
            &promotion.report_hash,
            &promotion.report_json,
            &stage_a.report_hash,
            &stage_a.report_json,
        ) else {
            tracing::warn!("late interaction activation evidence is invalid; serving baseline");
            return Ok(empty);
        };
        let profile_identity = match controller.identity().digest() {
            Ok(identity) => identity,
            Err(error) => {
                tracing::warn!(
                    "late interaction identity cannot be validated; serving baseline: {error}"
                );
                return Ok(empty);
            }
        };
        if record.validate_against_report(&report).is_err()
            || record.profile_identity != profile_identity.as_str()
            || record.generation_id != controller.identity().generation_id.value().to_string()
            || record.corpus_snapshot != controller.identity().corpus_snapshot.value().to_string()
        {
            tracing::warn!(
                "late interaction promotion is not bound to the active identity; serving baseline"
            );
            return Ok(empty);
        }
        Ok(record.promoted_classes)
    }
    pub(crate) fn late_interaction_shadow_engine(&self) -> Result<RetrievalEngine> {
        let reranker = self
            .late_interaction_shadow_reranker
            .clone()
            .ok_or_else(|| anyhow!("late interaction shadow reranker is not configured"))?;
        Ok(self
            .retrieval_engine()?
            .with_late_interaction_reranker(reranker))
    }
}
