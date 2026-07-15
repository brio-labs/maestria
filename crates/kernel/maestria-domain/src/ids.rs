use std::fmt;

pub const DOMAIN_VERSION: &str = "0.1.0";

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
id_type!(RetrievalModelFingerprintId);
id_type!(DuplicateClusterId);
id_type!(ConflictSetId);
