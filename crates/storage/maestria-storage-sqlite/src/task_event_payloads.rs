use super::event_payloads::StoredEventPayload;
use super::evidence_payloads::{StoredTaskPriority, StoredTaskStatus};
use maestria_domain::{ArtifactId, DomainEvent, TaskId, ValidationReportId};

impl StoredEventPayload {
    pub(crate) fn try_from_domain_task(event: &DomainEvent) -> Option<Self> {
        match event {
            DomainEvent::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => Some(Self::TaskOpened {
                task_id: task_id.value(),
                title: title.clone(),
                priority: StoredTaskPriority::from_domain(*priority),
                artifact_id: artifact_id.map(|id| id.value()),
            }),
            DomainEvent::TaskStatusChanged { task_id, from, to } => Some(Self::TaskStatusChanged {
                task_id: task_id.value(),
                from: StoredTaskStatus::from_domain(*from),
                to: StoredTaskStatus::from_domain(*to),
            }),
            DomainEvent::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => Some(Self::TaskCompletionRecorded {
                task_id: task_id.value(),
                status: StoredTaskStatus::from_domain(*status),
                validation_report_id: validation_report_id.value(),
            }),
            DomainEvent::UserIntentObserved { task_id, title } => Some(Self::UserIntentObserved {
                task_id: task_id.value(),
                title: title.clone(),
            }),
            DomainEvent::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Some(Self::HarnessRunCompleted {
                task_id: task_id.map(|id| id.value()),
                command: command.clone(),
                exit_code: *exit_code,
            }),
            DomainEvent::ApprovalRecorded { task_id, approved } => Some(Self::ApprovalRecorded {
                task_id: task_id.value(),
                approved: *approved,
            }),
            DomainEvent::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Some(Self::ValidationReportCreated {
                report_id: report_id.value(),
                task_id: task_id.map(|id| id.value()),
                passed: *passed,
                warnings: warnings.clone(),
            }),
            _ => None,
        }
    }

    pub(crate) fn try_into_domain_task(self) -> Result<DomainEvent, Box<Self>> {
        match self {
            Self::TaskOpened {
                task_id,
                title,
                priority,
                artifact_id,
            } => Ok(DomainEvent::TaskOpened {
                task_id: TaskId::new(task_id),
                title,
                priority: priority.into_domain(),
                artifact_id: artifact_id.map(ArtifactId::new),
            }),
            Self::TaskStatusChanged { task_id, from, to } => Ok(DomainEvent::TaskStatusChanged {
                task_id: TaskId::new(task_id),
                from: from.into_domain(),
                to: to.into_domain(),
            }),
            Self::TaskCompletionRecorded {
                task_id,
                status,
                validation_report_id,
            } => Ok(DomainEvent::TaskCompletionRecorded {
                task_id: TaskId::new(task_id),
                status: status.into_domain(),
                validation_report_id: ValidationReportId::new(validation_report_id),
            }),
            Self::UserIntentObserved { task_id, title } => Ok(DomainEvent::UserIntentObserved {
                task_id: TaskId::new(task_id),
                title,
            }),
            Self::HarnessRunCompleted {
                task_id,
                command,
                exit_code,
            } => Ok(DomainEvent::HarnessRunCompleted {
                task_id: task_id.map(TaskId::new),
                command,
                exit_code,
            }),
            Self::ApprovalRecorded { task_id, approved } => Ok(DomainEvent::ApprovalRecorded {
                task_id: TaskId::new(task_id),
                approved,
            }),
            Self::ValidationReportCreated {
                report_id,
                task_id,
                passed,
                warnings,
            } => Ok(DomainEvent::ValidationReportCreated {
                report_id: ValidationReportId::new(report_id),
                task_id: task_id.map(TaskId::new),
                passed,
                warnings,
            }),
            other => Err(Box::new(other)),
        }
    }

    pub(crate) fn try_kind_task(&self) -> Option<&'static str> {
        match self {
            Self::TaskOpened { .. } => Some("task_opened"),
            Self::TaskStatusChanged { .. } => Some("task_status_changed"),
            Self::TaskCompletionRecorded { .. } => Some("task_completion_recorded"),
            Self::UserIntentObserved { .. } => Some("user_intent_observed"),
            Self::HarnessRunCompleted { .. } => Some("harness_run_completed"),
            Self::ApprovalRecorded { .. } => Some("approval_recorded"),
            Self::ValidationReportCreated { .. } => Some("validation_report_created"),
            _ => None,
        }
    }

    pub(crate) fn try_filter_artifact_id_task(&self) -> Option<u64> {
        match self {
            Self::TaskOpened {
                artifact_id: Some(artifact_id),
                ..
            } => Some(*artifact_id),
            _ => None,
        }
    }
}
