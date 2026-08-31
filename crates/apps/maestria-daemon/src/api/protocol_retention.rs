//! Retrieval-retirement protocol payload (ADR-0009).

use serde::{Deserialize, Serialize};

/// Durable high-water of retrieval audit retirement after a successful
/// marker submission (ADR-0009).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalEventsRetiredResponse {
    pub retired_through: u64,
}
