#[path = "evidence_observability.rs"]
mod evidence_observability;
#[path = "index_observability.rs"]
mod index_observability;
#[path = "search_observability.rs"]
mod search_observability;
#[path = "search_render.rs"]
mod search_render;

use anyhow::{Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::DomainEventEnvelope;
use maestria_ports::{EventFilter, EventLog};
use maestria_storage_sqlite::SqliteStore;

pub use evidence_observability::run_evidence_coverage;
pub use index_observability::run_index_generations;
pub use search_observability::{run_search_compare, run_search_explain, run_search_trace};

pub(super) fn load_events(layout: &InstanceLayout) -> Result<Vec<DomainEventEnvelope>> {
    let store = SqliteStore::open(&layout.database_path)?;
    load_events_from_store(&store)
}

pub(super) fn load_events_from_store(store: &SqliteStore) -> Result<Vec<DomainEventEnvelope>> {
    store
        .scan(EventFilter { artifact_id: None })
        .map_err(|error| anyhow!("read durable event log: {error}"))
}
