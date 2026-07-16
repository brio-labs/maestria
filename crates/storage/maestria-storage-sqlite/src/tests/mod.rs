mod contract_tests;
mod event_tests;
mod id_allocator_tests;
mod index_event_tests;
mod index_generation_tests;
mod migration_tests;
mod repository_tests;

use std::collections::BTreeSet;

use maestria_domain::*;

pub(super) fn artifact(id: u64) -> Artifact {
    Artifact {
        id: ArtifactId::new(id),
        title: format!("artifact {id}"),
        chunk_ids: BTreeSet::new(),
        card_ids: BTreeSet::new(),
        claim_ids: BTreeSet::new(),
        evidence_ids: BTreeSet::new(),
        index_status: IndexStatus::default(),
        parse_status: None,
        content_hash: None,
        security: SecurityMetadata::default(),
    }
}

pub(super) fn registered(event_id: u64, sequence: u64, artifact_id: u64) -> DomainEventEnvelope {
    DomainEventEnvelope {
        id: EventId::new(event_id),
        sequence: SequenceNumber::new(sequence),
        event: DomainEvent::ArtifactRegistered {
            artifact_id: ArtifactId::new(artifact_id),
            title: format!("artifact {artifact_id}"),
            security: SecurityMetadata::default(),
        },
    }
}
mod effect_journal_tests;
