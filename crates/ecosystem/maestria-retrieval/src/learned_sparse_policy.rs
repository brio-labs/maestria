use crate::learned_sparse_benchmark::{
    LearnedSparsePromotionRecord, LearnedSparseQueryClass, LearnedSparseRoute,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum LearnedSparseExecutionPolicy {
    Disabled,
    #[default]
    Shadow,
    Active(LearnedSparsePromotionRecord),
}

impl LearnedSparseExecutionPolicy {
    pub fn route_for(&self, query: &str) -> LearnedSparseRoute {
        let class = LearnedSparseQueryClass::classify(query);
        match self {
            Self::Active(record) if record.is_valid() => {
                match record.winning_routes().get(&class).copied() {
                    Some(route) => route,
                    None => {
                        let _ = ();
                        LearnedSparseRoute::Hybrid
                    }
                }
            }
            Self::Disabled | Self::Shadow | Self::Active(_) => LearnedSparseRoute::Hybrid,
        }
    }

    pub fn allows_sparse(&self, query: &str) -> bool {
        matches!(
            self.route_for(query),
            LearnedSparseRoute::SparseOnly | LearnedSparseRoute::SparseFused
        )
    }

    pub fn should_shadow(&self, query: &str) -> bool {
        match self {
            Self::Shadow => true,
            Self::Active(record) if record.is_valid() => !self.allows_sparse(query),
            Self::Disabled | Self::Active(_) => false,
        }
    }
}

pub(crate) fn is_sparse_descriptor(descriptor: &crate::types::RetrieverDescriptor) -> bool {
    let id = descriptor.id.to_ascii_lowercase();
    descriptor
        .modality
        .to_ascii_lowercase()
        .starts_with("sparse")
        || id.contains("learned_sparse")
        || descriptor.representation.0 == maestria_ports::SPARSE_REPRESENTATION_V1
}

pub(crate) fn sparse_lane_is_eligible(
    descriptor: &crate::types::RetrieverDescriptor,
    sparse_enabled: bool,
) -> bool {
    if !is_sparse_descriptor(descriptor) {
        return true;
    }
    sparse_enabled && !descriptor.modality.eq_ignore_ascii_case("sparse-shadow")
}
