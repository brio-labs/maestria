use maestria_domain::*;

/// Count [`DomainEvent::ChunkRegistered`] events in output.
pub fn count_chunk_events(output: &KernelOutput) -> usize {
    output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ChunkRegistered { .. }))
        .count()
}

/// Count [`DomainEvent::CardCreated`] events in output.
pub fn count_card_events(output: &KernelOutput) -> usize {
    output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::CardCreated { .. }))
        .count()
}

/// Count [`DomainEvent::ArtifactParsed`] events in output.
pub fn count_parsed_events(output: &KernelOutput) -> usize {
    output
        .events
        .iter()
        .filter(|e| matches!(e.event, DomainEvent::ArtifactParsed { .. }))
        .count()
}
