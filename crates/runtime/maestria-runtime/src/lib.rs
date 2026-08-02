mod approval;
mod completion;
/// Responsibility map:
/// - `config`: runtime configuration and effect execution context.
/// - `effect_admission`: typed governance admission and stored-approval disposition.
/// - `effect_dispatch`: effect admission and reservation.
/// - `effect_execution`: effect execution handlers and retry lifecycle.
/// - `effect_execution_dispatch`: governance dispatch and effect routing.
/// - `effect_result`: effect failure and result handling.
/// - `harness`: harness execution effects.
/// - `harness_gate`: harness capability, scope, and path gating policy.
/// - `indexing`: full-text indexing effects.
/// - `ocr`: module responsibility.
/// - `parser_mapping`: parser status mappings.
/// - `parsing`: artifact parsing effects.
/// - `parsing_records`: parser record construction.
/// - `persistence`: event and entity persistence effects.
/// - `persistence_barrier`: durable event-log persistence barrier polling.
/// - `proposal_persistence`: approval continuation codec and pending-harness persistence.
/// - `proposal_recovery`: durable journal recovery boundary for model-agent harness outcomes.
/// - `proposal_workflow`: governed model-agent terminal workflow and results.
/// - `shell_policy`: shell grammar and scope policy.
/// - `supervision`: harness feedback supervision.
/// - `validation`: validation effects and report construction.
/// - `vector_indexing`: vector indexing effects.
/// - `web_evidence`: web evidence effects.
/// - `parsing_terminal`: terminal parsing boundary.
/// - `approval`: approval boundary validation.
/// - `completion`: task completion boundary validation.
/// - `runtime`: runtime public types and errors.
/// - `runtime_effects`: concurrent effect executor lifecycle.
/// - `runtime_handle`: runtime handle command and feedback submission.
/// - `runtime_loop`: runtime lifecycle and command loop.
/// - `runtime_transition`: staged transition metadata and persistence barriers.
mod config;
mod effect_admission;
mod effect_dispatch;
mod effect_execution;
mod effect_execution_dispatch;
mod effect_result;
mod harness;
mod harness_gate;
mod indexing;
mod ocr;
mod parser_mapping;
mod parsing;
mod parsing_records;
mod parsing_terminal;
mod persistence;
mod persistence_barrier;
mod proposal_persistence;
mod proposal_recovery;
mod proposal_workflow;
mod runtime;
mod runtime_effects;
mod runtime_handle;
mod runtime_loop;
mod runtime_transition;
mod shell_policy;
mod supervision;
mod validation;
mod vector_indexing;
mod web_evidence;

pub use config::{Adapters, Governance, RuntimeConfig};
pub use proposal_persistence::decode_pending_continuation;
pub use runtime::{
    DomainApplicationResult, FeedbackError, MaestriaRuntime, RuntimeHandle, RuntimeSubmissionError,
    RuntimeSubmissionPermit,
};

#[cfg(test)]
pub use config::EffectExecutionContext;
#[cfg(test)]
mod test_helpers;
#[cfg(test)]
pub mod test_support;
#[cfg(test)]
mod tests;
