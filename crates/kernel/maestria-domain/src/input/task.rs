use crate::types::*;

impl KernelState {
    // ── Handlers ─────────────────────────────────────────────────

    pub(super) fn handle_open_task(
        &mut self,
        input: OpenTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        if self.tasks.contains_key(&input.task_id) {
            return Err(DomainError::DuplicateId {
                kind: "task",
                id: input.task_id.value(),
            });
        }
        if let Some(artifact_id) = input.artifact_id
            && !self.artifacts.contains_key(&artifact_id)
        {
            return Err(DomainError::MissingArtifact { id: artifact_id });
        }

        let task = Task::new(input.task_id, input.title.clone(), input.priority);
        let artifact_id = input.artifact_id;
        self.tasks.insert(input.task_id, task);
        if let Some(artifact_id) = artifact_id
            && let Some(task) = self.tasks.get_mut(&input.task_id)
        {
            task.artifact_ids.insert(artifact_id);
        }

        Ok(self.emit_event(DomainEvent::TaskOpened {
            task_id: input.task_id,
            title: input.title,
            priority: input.priority,
            artifact_id: input.artifact_id,
        }))
    }

    pub(super) fn handle_change_task_status(
        &mut self,
        task_id: TaskId,
        to: TaskStatus,
    ) -> Result<(TaskStatus, TaskStatus), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        let from = task.status;
        if to.is_completion() {
            return Err(DomainError::ValidationRequired { task_id });
        }
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition { task_id, from, to });
        }
        task.status = to;
        Ok((from, to))
    }

    pub(super) fn handle_complete_task(
        &mut self,
        input: CompleteTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let task = self
            .tasks
            .get_mut(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;
        let report = self
            .validation_reports
            .get(&input.validation_report_id)
            .ok_or(DomainError::MissingValidationReport {
                id: input.validation_report_id,
            })?;
        if report.task_id != Some(input.task_id) {
            return Err(DomainError::ValidationReportTaskMismatch {
                report_id: input.validation_report_id,
                report_task_id: report.task_id,
                task_id: input.task_id,
            });
        }
        let from = task.status;
        if !report.passed {
            return Err(DomainError::ValidationFailed {
                task_id: input.task_id,
            });
        }
        let to = if report.warnings.is_empty() {
            TaskStatus::CompletedVerified
        } else {
            TaskStatus::CompletedWithWarnings
        };
        if !from.can_transition_to(to) {
            return Err(DomainError::InvalidTaskTransition {
                task_id: input.task_id,
                from,
                to,
            });
        }
        task.status = to;
        task.validation_report_id = Some(input.validation_report_id);
        Ok(self.emit_event(DomainEvent::TaskCompletionRecorded {
            task_id: input.task_id,
            status: to,
            validation_report_id: input.validation_report_id,
        }))
    }

    pub(super) fn handle_link_evidence_to_task(
        &mut self,
        input: LinkEvidenceToTaskInput,
    ) -> Result<DomainEventEnvelope, DomainError> {
        let task = self
            .tasks
            .get_mut(&input.task_id)
            .ok_or(DomainError::MissingTask { id: input.task_id })?;
        if !self.evidences.contains_key(&input.evidence_id) {
            return Err(DomainError::MissingEvidence {
                id: input.evidence_id,
            });
        }

        task.evidence_ids.insert(input.evidence_id);

        Ok(self.emit_event(DomainEvent::TaskEvidenceLinked {
            task_id: input.task_id,
            evidence_id: input.evidence_id,
        }))
    }

    // ── Replay apply ─────────────────────────────────────────────

    pub(crate) fn apply_task_opened(
        &mut self,
        task_id: TaskId,
        title: &str,
        priority: TaskPriority,
        artifact_id: Option<ArtifactId>,
    ) -> Result<(), DomainError> {
        if self.tasks.contains_key(&task_id) {
            return Err(DomainError::DuplicateId {
                kind: "task",
                id: task_id.value(),
            });
        }
        if let Some(art_id) = artifact_id
            && !self.artifacts.contains_key(&art_id)
        {
            return Err(DomainError::MissingArtifact { id: art_id });
        }
        let mut task = Task::new(task_id, title.to_string(), priority);
        if let Some(art_id) = artifact_id {
            task.artifact_ids.insert(art_id);
        }
        self.tasks.insert(task_id, task);
        Ok(())
    }

    pub(crate) fn apply_task_status_changed(
        &mut self,
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    ) -> Result<(), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        if task.status != from {
            return Err(DomainError::InvalidTaskTransition {
                task_id,
                from: task.status,
                to: from,
            });
        }
        if from == to && !from.is_completion() {
            return Err(DomainError::InvalidTaskTransition { task_id, from, to });
        }
        if from != to {
            if to.is_completion() {
                return Err(DomainError::ValidationRequired { task_id });
            }
            if !from.can_transition_to(to) {
                return Err(DomainError::InvalidTaskTransition { task_id, from, to });
            }
            task.status = to;
        }
        Ok(())
    }

    pub(crate) fn apply_task_completion_recorded(
        &mut self,
        task_id: TaskId,
        status: TaskStatus,
        validation_report_id: ValidationReportId,
    ) -> Result<(), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        let report = self.validation_reports.get(&validation_report_id).ok_or(
            DomainError::MissingValidationReport {
                id: validation_report_id,
            },
        )?;
        if report.task_id != Some(task_id) {
            return Err(DomainError::ValidationReportTaskMismatch {
                report_id: validation_report_id,
                report_task_id: report.task_id,
                task_id,
            });
        }
        if !report.passed {
            return Err(DomainError::ValidationFailed { task_id });
        }
        if status == TaskStatus::CompletedVerified && !report.warnings.is_empty() {
            return Err(DomainError::ValidationWarningsForbidden { task_id });
        }
        if status == TaskStatus::CompletedWithWarnings && report.warnings.is_empty() {
            return Err(DomainError::ValidationWarningsRequired { task_id });
        }
        if status != TaskStatus::CompletedVerified && status != TaskStatus::CompletedWithWarnings {
            return Err(DomainError::InvalidTaskTransition {
                task_id,
                from: task.status,
                to: status,
            });
        }
        if !task.status.can_transition_to(status) {
            return Err(DomainError::InvalidTaskTransition {
                task_id,
                from: task.status,
                to: status,
            });
        }
        task.status = status;
        task.validation_report_id = Some(validation_report_id);
        Ok(())
    }

    pub(crate) fn apply_task_evidence_linked(
        &mut self,
        task_id: TaskId,
        evidence_id: EvidenceId,
    ) -> Result<(), DomainError> {
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(DomainError::MissingTask { id: task_id })?;
        if !self.evidences.contains_key(&evidence_id) {
            return Err(DomainError::MissingEvidence { id: evidence_id });
        }
        task.evidence_ids.insert(evidence_id);
        Ok(())
    }
}
