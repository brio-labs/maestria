use crate::MaestriaRuntime;
use maestria_domain::{ApprovalDecision, ModelAgentProposalExecution, ModelAgentProposalRequest};
use maestria_ports::{ApprovalRecord, ApprovalStatus};

impl MaestriaRuntime {
    pub(crate) async fn check_approval_boundary(&self, decision: &ApprovalDecision) -> bool {
        let Some(record) = self.lookup_pending_approval(decision) else {
            return false;
        };
        if !Self::validate_approval_task(decision, &record) {
            return false;
        }
        let Some(proposal) = Self::decode_approval_proposal(decision, &record) else {
            return false;
        };
        let Some(proposal) = proposal else {
            return true;
        };
        self.verify_approval_continuation(decision, &record, &proposal)
            .await
    }

    fn lookup_pending_approval(&self, decision: &ApprovalDecision) -> Option<ApprovalRecord> {
        let record = match self
            .adapters
            .approval_repo
            .find_by_id(decision.approval_id())
        {
            Ok(Some(record)) => record,
            Ok(None) => {
                tracing::warn!(
                    approval_id = %decision.approval_id(),
                    "approval resolve rejected: record not found"
                );
                return None;
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    approval_id = %decision.approval_id(),
                    "approval resolve rejected: repo lookup error"
                );
                return None;
            }
        };
        if record.status != ApprovalStatus::Pending {
            tracing::info!(
                approval_id = %decision.approval_id(),
                status = ?record.status,
                "approval resolve skipped: already resolved (idempotent)"
            );
            return None;
        }
        Some(record)
    }

    fn validate_approval_task(decision: &ApprovalDecision, record: &ApprovalRecord) -> bool {
        if let Some(task_id) = decision.task_id()
            && record.task_id != Some(task_id)
        {
            tracing::warn!(
                approval_id = %decision.approval_id(),
                record_task = ?record.task_id,
                input_task = ?task_id,
                "approval resolve rejected: task_id mismatch"
            );
            return false;
        }
        true
    }

    fn decode_approval_proposal(
        decision: &ApprovalDecision,
        record: &ApprovalRecord,
    ) -> Option<Option<ModelAgentProposalRequest>> {
        match crate::proposal_persistence::decode_pending_continuation(record) {
            Ok(Some(proposal)) => Some(Some(proposal)),
            Ok(None) => {
                tracing::info!(
                    approval_id = %decision.approval_id(),
                    "approval resolve accepted without pending continuation"
                );
                Some(None)
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    approval_id = %decision.approval_id(),
                    "approval resolve rejected: corrupt pending continuation"
                );
                None
            }
        }
    }

    async fn verify_approval_continuation(
        &self,
        decision: &ApprovalDecision,
        record: &ApprovalRecord,
        proposal: &ModelAgentProposalRequest,
    ) -> bool {
        if !self.verify_continuation_metadata(decision, record, proposal) {
            return false;
        }
        self.verify_stored_proposal_request(decision, proposal)
            .await
    }

    fn verify_continuation_metadata(
        &self,
        decision: &ApprovalDecision,
        record: &ApprovalRecord,
        proposal: &ModelAgentProposalRequest,
    ) -> bool {
        let ModelAgentProposalExecution::ApprovalContinuation {
            approval_id,
            journal_generation: _,
        } = &proposal.execution
        else {
            tracing::warn!(
                approval_id = %decision.approval_id(),
                "approval resolve rejected: continuation has invalid execution mode"
            );
            return false;
        };
        if *approval_id != decision.approval_id()
            || record.effect_kind != "model_agent_harness"
            || record.scope_id != self.config.scope_id
            || record
                .capability
                .strip_prefix("model_agent_pending:")
                .is_none()
        {
            tracing::warn!(
                approval_id = %decision.approval_id(),
                "approval resolve rejected: stored approval metadata mismatch"
            );
            return false;
        }
        true
    }

    async fn verify_stored_proposal_request(
        &self,
        decision: &ApprovalDecision,
        proposal: &ModelAgentProposalRequest,
    ) -> bool {
        let state = self.state.read().await;
        let Some(stored) = state.model_agent_requests.get(&proposal.run_id) else {
            tracing::warn!(
                approval_id = %decision.approval_id(),
                run_id = %proposal.run_id,
                "approval resolve rejected: stored model-agent request missing"
            );
            return false;
        };
        if !matches!(&stored.execution, ModelAgentProposalExecution::Fresh) {
            tracing::warn!(
                approval_id = %decision.approval_id(),
                run_id = %proposal.run_id,
                "approval resolve rejected: stored request is not fresh"
            );
            return false;
        }
        let mut expected = stored.clone();
        expected.execution = proposal.execution.clone();
        if expected != *proposal {
            tracing::warn!(
                approval_id = %decision.approval_id(),
                run_id = %proposal.run_id,
                "approval resolve rejected: continuation identity mismatch"
            );
            return false;
        }
        true
    }
}
