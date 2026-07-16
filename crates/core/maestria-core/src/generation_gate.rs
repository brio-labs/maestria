use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use maestria_domain::{DomainEvent, DomainEventEnvelope, EventId, SequenceNumber, replay_events};
use maestria_ports::EventFilter;

pub(crate) fn ensure_generation_is_serveable(
    ports: &CorePorts<'_>,
    generation_id: maestria_domain::IndexGenerationId,
) -> CoreResult<()> {
    let events = ports.events.scan(EventFilter { artifact_id: None })?;
    let generation_events: Vec<_> = events
        .into_iter()
        .filter(|envelope| {
            matches!(
                envelope.event,
                DomainEvent::IndexGenerationStarted { .. }
                    | DomainEvent::IndexGenerationTransitioned { .. }
            )
        })
        .enumerate()
        .map(|(index, envelope)| DomainEventEnvelope {
            id: EventId::new(index as u64 + 1),
            sequence: SequenceNumber::new(index as u64 + 1),
            event: envelope.event,
        })
        .collect();
    if generation_events.is_empty() {
        return Ok(());
    }
    let state = replay_events(&generation_events).map_err(|error| CoreError::InvalidInput {
        message: format!("invalid index generation state: {error}"),
    })?;
    if !state.index_generations.is_serveable(generation_id) {
        return Err(CoreError::InvalidInput {
            message: format!("index generation {} is not active", generation_id.value()),
        });
    }
    Ok(())
}
