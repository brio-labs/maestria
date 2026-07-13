use maestria_domain::RelationEndpoint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredRelationEndpoint {
    Artifact { artifact_id: u64 },
    Claim { claim_id: u64 },
    Task { task_id: u64 },
    Memory { memory_id: u64 },
    Card { card_id: u64 },
}

impl StoredRelationEndpoint {
    pub(crate) fn from_domain(endpoint: &RelationEndpoint) -> Self {
        match endpoint {
            RelationEndpoint::Artifact(id) => Self::Artifact {
                artifact_id: id.value(),
            },
            RelationEndpoint::Claim(id) => Self::Claim {
                claim_id: id.value(),
            },
            RelationEndpoint::Task(id) => Self::Task {
                task_id: id.value(),
            },
            RelationEndpoint::Memory(id) => Self::Memory {
                memory_id: id.value(),
            },
            RelationEndpoint::Card(id) => Self::Card {
                card_id: id.value(),
            },
        }
    }

    pub(crate) fn into_domain(self) -> RelationEndpoint {
        match self {
            Self::Artifact { artifact_id } => {
                RelationEndpoint::Artifact(maestria_domain::ArtifactId::new(artifact_id))
            }
            Self::Claim { claim_id } => {
                RelationEndpoint::Claim(maestria_domain::ClaimId::new(claim_id))
            }
            Self::Task { task_id } => RelationEndpoint::Task(maestria_domain::TaskId::new(task_id)),
            Self::Memory { memory_id } => {
                RelationEndpoint::Memory(maestria_domain::MemoryId::new(memory_id))
            }
            Self::Card { card_id } => RelationEndpoint::Card(maestria_domain::CardId::new(card_id)),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredRelationKind {
    Contains,
    Defines,
    Supports,
    Contradicts,
    UsedEvidence,
    BasedOn,
    DerivedFrom,
    AppliesTo,
    RelatedTo,
}

impl StoredRelationKind {
    pub(crate) fn from_domain(kind: &maestria_domain::RelationKind) -> Self {
        match kind {
            maestria_domain::RelationKind::Contains => Self::Contains,
            maestria_domain::RelationKind::Defines => Self::Defines,
            maestria_domain::RelationKind::Supports => Self::Supports,
            maestria_domain::RelationKind::Contradicts => Self::Contradicts,
            maestria_domain::RelationKind::UsedEvidence => Self::UsedEvidence,
            maestria_domain::RelationKind::BasedOn => Self::BasedOn,
            maestria_domain::RelationKind::DerivedFrom => Self::DerivedFrom,
            maestria_domain::RelationKind::AppliesTo => Self::AppliesTo,
            maestria_domain::RelationKind::RelatedTo => Self::RelatedTo,
        }
    }

    pub(crate) fn into_domain(self) -> maestria_domain::RelationKind {
        match self {
            Self::Contains => maestria_domain::RelationKind::Contains,
            Self::Defines => maestria_domain::RelationKind::Defines,
            Self::Supports => maestria_domain::RelationKind::Supports,
            Self::Contradicts => maestria_domain::RelationKind::Contradicts,
            Self::UsedEvidence => maestria_domain::RelationKind::UsedEvidence,
            Self::BasedOn => maestria_domain::RelationKind::BasedOn,
            Self::DerivedFrom => maestria_domain::RelationKind::DerivedFrom,
            Self::AppliesTo => maestria_domain::RelationKind::AppliesTo,
            Self::RelatedTo => maestria_domain::RelationKind::RelatedTo,
        }
    }
}
