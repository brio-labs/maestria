use maestria_domain::{
    ApprovalId, ArtifactId, CardId, ChunkId, ClaimId, EventId, EvidenceId, HarnessRunId,
    IndexGenerationId, LogicalTick, MemoryId, NotebookDraftId, NotebookId, RelationId, ScopeId,
    StructureNodeId, TaskId, ValidationReportId,
};
use maestria_ports::PortError;

/// Converts a u64 id to SQLite INTEGER, rejecting values outside the i64
/// range with an invalid-input error.
pub fn u64_to_i64(value: u64) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| {
        PortError::invalid_input("identifier exceeds sqlite INTEGER range", value.to_string())
    })
}

/// Converts a u64 id stored as SQLite INTEGER back, rejecting negative
/// stored values as an internal error.
pub fn i64_to_u64(value: i64) -> Result<u64, PortError> {
    u64::try_from(value)
        .map_err(|_| PortError::internal("stored identifier is negative", value.to_string()))
}

/// Converts a u32 id stored as SQLite INTEGER back.
pub fn i64_to_u32(value: i64) -> Result<u32, PortError> {
    u32::try_from(value).map_err(|_| {
        PortError::internal("stored chunk order is outside u32 range", value.to_string())
    })
}

/// Converts a usize stored as SQLite INTEGER back.
pub fn i64_to_usize(value: i64) -> Result<usize, PortError> {
    usize::try_from(value)
        .map_err(|_| PortError::internal("stored value is negative", value.to_string()))
}

/// Converts a usize bound (e.g. dimension, byte count) to SQLite INTEGER.
pub fn usize_to_i64(value: usize) -> Result<i64, PortError> {
    i64::try_from(value).map_err(|_| {
        PortError::invalid_input("value exceeds sqlite INTEGER range", value.to_string())
    })
}

/// Optional variant of [`u64_to_i64`].
pub fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>, PortError> {
    value.map(u64_to_i64).transpose()
}

/// Optional variant of [`i64_to_u64`].
pub fn optional_i64_to_u64(value: Option<i64>) -> Result<Option<u64>, PortError> {
    value.map(i64_to_u64).transpose()
}

/// SQL binding for domain ids: converts `self.value()` to SQLite INTEGER.
///
/// Use in `params!` lists so the conversion and its error mapping stay in one
/// place: `params![id.to_sql_param()?]` instead of
/// `params![u64_to_i64(id.value())?]`.
pub trait BindId {
    fn to_sql_param(&self) -> Result<i64, PortError>;
}

/// Implements [`BindId`] for a list of domain id newtypes.
macro_rules! impl_bind_id {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl BindId for $ty {
                fn to_sql_param(&self) -> Result<i64, PortError> {
                    u64_to_i64(self.value())
                }
            }
        )+
    };
}

impl_bind_id!(
    ApprovalId,
    ArtifactId,
    CardId,
    ChunkId,
    ClaimId,
    EventId,
    EvidenceId,
    IndexGenerationId,
    LogicalTick,
    MemoryId,
    HarnessRunId,
    NotebookDraftId,
    NotebookId,
    RelationId,
    ScopeId,
    StructureNodeId,
    TaskId,
    ValidationReportId,
);
