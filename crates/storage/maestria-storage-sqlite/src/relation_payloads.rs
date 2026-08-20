use maestria_domain::{RelationEndpoint, RelationKind};
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

crate::stored_enum! {
    #[serde(rename_all = "snake_case")]
    pub(crate) enum StoredRelationKind <=> RelationKind {
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
}
