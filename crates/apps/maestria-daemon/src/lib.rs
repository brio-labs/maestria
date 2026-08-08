/// Responsibility map:
/// - `api`: daemon protocol, client, server, and service handlers.
/// - `lock`: exclusive instance write-lock acquisition.
/// - `search_executor`: search runtime assembly and execution.
/// - `approval_recovery`: approval repository reconciliation.
/// - `projection_recovery`: graph and vector projection recovery.
/// - `vector_startup`: embedding generation activation and vector startup.
/// - `full_text_recovery`: pending full-text recovery inputs.
/// - `parser_resume`: pending parser blob verification.
/// - `projection_open`: shared store and projection opening for runtime construction.
/// - `recovery_inputs`: recovery input collection and ordering.
/// - `recovery_staging`: startup recovery queue staging and event-log scanning.
/// - `supervision_recovery`: supervised recovery diagnostics.
/// - `validation_recovery`: validation report recovery checks.
/// - `lifecycle`: instance runtime lifecycle and recovery queue.
/// - `mutation_session`: ready, correlated one-shot mutation lifecycle.
/// - `watcher`: filesystem watcher ingestion.
/// - `lifecycle_entry`: lifecycle entrypoint orchestration.
/// - `instance_setup`: instance initialization, replay, and recovery scope validation.
/// - `providers`: OCR and visual provider construction and status.
/// - `learned_sparse_benchmark_executor`: real-instance four-profile benchmark execution.
/// - `sparse_startup`: learned-sparse generation and projection reconciliation.
/// - `runtime_construction`: runtime adapter and governance assembly.
/// - `blocked_patterns`: blocked-path composition for runtime construction.
/// - `db_retry`: shared database-busy retry policy.
/// - `evidence_open`: shared read-only evidence store assembly.
/// - `notebook_draft_open`: draft blob persistence and opening.
/// - `ingestion_policy`: shared source-file and privacy exclusion policy.
/// - `source_identity`: canonical source-path identity keys for watcher and recovery.
pub mod api;
mod approval_recovery;
pub mod blocked_patterns;
pub mod db_retry;
pub mod evidence_open;
mod full_text_recovery;
pub mod ingestion_policy;
mod instance_setup;
#[cfg(test)]
#[path = "learned_sparse_activation_tests.rs"]
mod learned_sparse_activation_tests;
mod learned_sparse_benchmark_executor;
mod lifecycle;
mod lifecycle_entry;
mod lock;
mod mutation_session;
mod notebook_draft_open;
mod parser_resume;
mod projection_open;
mod projection_recovery;
mod providers;
mod recovery_inputs;
mod recovery_staging;
mod runtime_construction;
mod search_executor;
mod source_identity;
mod sparse_startup;
mod supervision_recovery;
#[cfg(test)]
mod test_support;
mod validation_recovery;
mod vector_startup;
mod watcher;

pub use api::{
    ApiServer, ClientAuthentication, ClientOperation, ClientRequest, ClientResponse, DaemonClient,
    FederationCredential, FrozenNotebookCitationResponse, NotebookCitationResponse,
    NotebookContextResponse, NotebookDraftDeletedResponse, NotebookDraftListResponse,
    NotebookDraftResponse, NotebookDraftSavedResponse, NotebookDraftSummary, NotebookListResponse,
    NotebookResponse, NotebookSourceCatalogEntry, NotebookSourceCatalogResponse,
    NotebookSourceSelection, NotebookSummary, RealmGrantAccess, RealmGrantSensitivity,
};
pub use approval_recovery::{reconcile_approval_repo, reconcile_pending_approvals};
pub use full_text_recovery::pending_start_full_text;
pub use instance_setup::{
    load_kernel_state, prepare_instance, prepare_instance_with_roots, validate_recovery_scope,
};
pub use learned_sparse_benchmark_executor::LearnedSparseBenchmarkExecutor;
pub(crate) use lifecycle::InstanceLifecycle;
pub use lifecycle::RecoveryQueue;
pub use lifecycle_entry::{run_instance, run_instance_with_shutdown};
pub use lock::{
    InstanceWriteLock, acquire as acquire_instance_write_lock,
    try_acquire as try_acquire_instance_write_lock,
};
pub use mutation_session::MutationSession;
pub use parser_resume::verify_pending_blobs;
pub use projection_recovery::{
    reconcile_graph_projection, reconcile_projections, reconcile_vector_projection,
};
pub use providers::{
    build_sparse_provider, build_visual_provider, ocr_status, sparse_status, visual_status,
};
pub use recovery_inputs::{RecoveryInputs, recovery_inputs};
pub use search_executor::{
    SearchRuntime, prepare_search_runtime, prepare_search_runtime_read_only,
    prepare_search_runtime_read_only_for_federation,
    prepare_search_runtime_read_only_with_repository_policy,
    prepare_search_runtime_with_repository_policy,
};
pub use sparse_startup::{
    build_sparse_provider_for_layout, reconcile_sparse_generation,
    reconcile_sparse_projection_for_layout, sparse_fingerprint, sparse_identity, sparse_namespace,
};
pub use supervision_recovery::{RecoveryDiagnostics, supervise_recovery};
pub use validation_recovery::has_current_validation_report;
pub use vector_startup::{
    RetrievalGenerations, build_embedding_provider, reconcile_retrieval_generations,
    reconcile_vector_projection_for_layout,
};
