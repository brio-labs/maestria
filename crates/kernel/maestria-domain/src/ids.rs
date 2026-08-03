use std::fmt;

pub const DOMAIN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The single scope owned by a Maestria instance (R43). Instances are
/// single-scope by construction: the runtime effect path, direct CLI/API
/// search surfaces, and the evidence open path all confine to this scope.
pub const DEFAULT_INSTANCE_SCOPE_ID: ScopeId = ScopeId::new(1);

/// The single corpus snapshot served by an instance. Index generations are
/// reconciled against this snapshot (R9: projections are rebuildable, the
/// snapshot identifies the served corpus), so capabilities claim exactly
/// this snapshot instead of a fabricated default.
pub const DEFAULT_CORPUS_SNAPSHOT_ID: CorpusSnapshotId = CorpusSnapshotId::new(1);

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Debug,
            Clone,
            Copy,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            ::serde::Serialize,
            ::serde::Deserialize,
        )]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn value(&self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(ArtifactId);
id_type!(ChunkId);
id_type!(CardId);
id_type!(EvidenceId);
id_type!(ClaimId);
id_type!(TaskId);
id_type!(EventId);
id_type!(SequenceNumber);
id_type!(SnapshotId);
id_type!(LogicalTick);
id_type!(RelationId);
id_type!(MemoryCandidateId);
id_type!(MemoryId);
id_type!(ValidationReportId);
id_type!(ApprovalId);
id_type!(HarnessRunId);
id_type!(BlobId);
id_type!(ScopeId);
id_type!(ArtifactVersionId);
id_type!(StructureNodeId);
id_type!(QueryId);
id_type!(SearchTraceId);
id_type!(CorpusSnapshotId);
id_type!(IndexGenerationId);
id_type!(DuplicateClusterId);
id_type!(ConflictSetId);
id_type!(CorrelationId);
id_type!(JournalGeneration);

impl QueryId {
    /// Deterministic query identity derived from the query text, so plans,
    /// traces, evidence packs, and shadow observations for the same query
    /// share one identity while distinct queries never collapse to a single
    /// id (R42: search traces identify the query). Same FNV-1a mixing used by
    /// `SearchTrace::deterministic_id`.
    pub fn from_query_text(query: &str) -> Self {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in query.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Self::new(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_id_is_deterministic_per_query_text() {
        assert_eq!(
            QueryId::from_query_text("alpha token"),
            QueryId::from_query_text("alpha token")
        );
    }

    #[test]
    fn distinct_queries_do_not_collapse_to_one_id() {
        assert_ne!(
            QueryId::from_query_text("alpha token"),
            QueryId::from_query_text("beta token")
        );
        assert_ne!(
            QueryId::from_query_text("alpha token"),
            QueryId::from_query_text("alpha token?")
        );
    }
}
