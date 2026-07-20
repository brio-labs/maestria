use std::sync::Arc;

use crate::traits::CandidateRetriever;

pub(super) fn capabilities_from_retrievers(
    retrievers: &[Arc<dyn CandidateRetriever>],
) -> maestria_governance::SearchCapabilities {
    use maestria_domain::{
        CorpusSnapshotId, IndexGenerationId, Modality, SearchIntent, SearchStage,
    };

    let primary_generation = retrievers
        .iter()
        .map(|retriever| retriever.descriptor())
        .find(|descriptor| !descriptor.modality.eq_ignore_ascii_case("dense"))
        .map_or(IndexGenerationId::new(1), |descriptor| {
            descriptor.generation
        });
    let mut capabilities = maestria_governance::SearchCapabilities::new()
        .with_intent(SearchIntent::ExactLookup)
        .with_intent(SearchIntent::FactualLocal)
        .with_stage(SearchStage::InitialRetrieval)
        .with_snapshot(CorpusSnapshotId::new(1))
        .with_generation(primary_generation)
        .allow_global_scope()
        .max_scope_ids(u32::MAX)
        .max_budgets(1_000, 30_000, 8, 3, 0)
        .with_security_filters();
    let mut known_modality = false;
    for retriever in retrievers {
        match retriever
            .descriptor()
            .modality
            .to_ascii_lowercase()
            .as_str()
        {
            "text" | "lexical" => {
                capabilities = capabilities.with_modality(Modality::Text);
                known_modality = true;
            }
            "code" | "rust" => {
                capabilities = capabilities
                    .with_modality(Modality::Code)
                    .with_intent(SearchIntent::RepositoryCode);
                known_modality = true;
            }
            "image" => {
                capabilities = capabilities
                    .with_modality(Modality::Image)
                    .with_intent(SearchIntent::VisualDocument);
                known_modality = true;
            }
            "pdf" => {
                capabilities = capabilities.with_modality(Modality::Pdf);
                known_modality = true;
            }
            "table" => {
                capabilities = capabilities.with_modality(Modality::Table);
                known_modality = true;
            }
            "web" => {
                capabilities = capabilities
                    .with_modality(Modality::Web)
                    .with_intent(SearchIntent::CurrentWeb)
                    .enable_web()
                    .support_realtime()
                    .max_budgets(1_000, 30_000, 8, 3, 1);
                known_modality = true;
            }
            "vector" | "dense" | "semantic" | "sparse" | "sparse-shadow"
            | "learned_sparse" => {
                capabilities = capabilities
                    .with_modality(Modality::Text)
                    .with_intent(SearchIntent::SemanticDiscovery);
                known_modality = true;
            }
            _ => {}
        }
    }
    if !known_modality {
        capabilities = capabilities.with_modality(Modality::Text);
    }
    capabilities
}

pub(crate) fn batch_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    hybrid_policy: &crate::types::HybridExecutionPolicy,
    repository_specialized: bool,
) -> bool {
    let id = descriptor.id.to_ascii_lowercase();
    let is_dense = id.contains("dense") || id.contains("vector") || id.contains("semantic");
    let is_code = descriptor.modality.eq_ignore_ascii_case("code")
        || descriptor.modality.eq_ignore_ascii_case("rust")
        || id.contains("code_intel");
    let hybrid_allowed = match hybrid_policy {
        crate::types::HybridExecutionPolicy::Shadow => !is_dense,
        crate::types::HybridExecutionPolicy::Active(_) => true,
    };
    hybrid_allowed && (repository_specialized || !is_code)
}
