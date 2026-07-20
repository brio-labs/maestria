use crate::learned_sparse_benchmark::{
    LearnedSparsePromotionRecord, LearnedSparseQueryClass, LearnedSparseRoute,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LearnedSparseExecutionPolicy {
    #[default]
    Shadow,
    Active(LearnedSparsePromotionRecord),
}

impl LearnedSparseExecutionPolicy {
    pub fn route_for(&self, query: &str) -> LearnedSparseRoute {
        let class = LearnedSparseQueryClass::classify(query);
        match self {
            Self::Active(record) if record.is_valid() => record
                .winning_routes()
                .get(&class)
                .copied()
                .unwrap_or(LearnedSparseRoute::Hybrid),
            Self::Shadow | Self::Active(_) => LearnedSparseRoute::Hybrid,
        }
    }

    pub fn allows_sparse(&self, query: &str) -> bool {
        matches!(
            self.route_for(query),
            LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused
        )
    }
}

pub(crate) fn sparse_lane_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    sparse_enabled: bool,
) -> bool {
    let id = descriptor.id.to_ascii_lowercase();
    let is_sparse = descriptor.modality.eq_ignore_ascii_case("sparse")
        || id.contains("learned_sparse")
        || descriptor.representation.0 == maestria_ports::SPARSE_REPRESENTATION_V1;
    sparse_enabled || !is_sparse
}
