use anyhow::{Context, Result, anyhow};
use maestria_core::InstanceLayout;
use maestria_domain::{Evidence, EvidenceKind, KernelState, Task};

use super::super::protocol::{
    ClientOperation, ClientResponse, EvidenceResponse, EvidenceSourceResponse, StatusResponse,
    TaskResponse, TaskSummary,
};
use super::super::server::ApiContext;
use super::support;

pub(super) fn status(
    layout: &InstanceLayout,
    socket_path: &std::path::Path,
) -> Result<StatusResponse> {
    let state = load_state(layout)?;
    Ok(StatusResponse {
        instance_root: layout.root.display().to_string(),
        event_count: state.event_log.len(),
        task_count: state.tasks.len(),
        socket_path: socket_path.display().to_string(),
    })
}

pub(super) fn task(layout: &InstanceLayout, task_id: Option<u64>) -> Result<TaskResponse> {
    let state = load_state(layout)?;
    let tasks: Vec<TaskSummary> = state
        .tasks
        .iter()
        .filter(|(id, _)| task_id.is_none_or(|requested| id.value() == requested))
        .map(|(_, task)| task_summary(task))
        .collect();
    if task_id.is_some() && tasks.is_empty() {
        return Err(anyhow!("task not found"));
    }
    Ok(TaskResponse { tasks })
}

pub(super) fn open_evidence(layout: &InstanceLayout, evidence_id: u64) -> Result<EvidenceResponse> {
    // Single owner of evidence scope + policy enforcement (R28/R48): the
    // shared scoped open applies the manifest read-root scope and retrieval
    // policy before dispatching to core.
    let output = crate::evidence_open::open_evidence_scoped(layout, evidence_id)?;
    evidence_response(output)
}

pub(super) fn evidence_response(
    output: maestria_core::OpenEvidenceOutput,
) -> Result<EvidenceResponse> {
    Ok(EvidenceResponse {
        evidence_id: output.evidence.id.value(),
        artifact_id: output.artifact.id.value(),
        artifact_title: output.artifact.title,
        artifact_content_hash: output
            .artifact
            .content_hash
            .map(|hash| hash.as_str().to_owned()),
        source: evidence_source(&output.evidence)?,
        excerpt: output.evidence.excerpt,
        observed_at: output.evidence.observed_at.value(),
    })
}

fn load_state(layout: &InstanceLayout) -> Result<KernelState> {
    crate::instance_setup::load_kernel_state(layout)
}

fn evidence_source(evidence: &Evidence) -> Result<EvidenceSourceResponse> {
    Ok(match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            snapshot,
        } => EvidenceSourceResponse::File {
            path: path.clone(),
            start_line: u32::try_from(range.start())
                .context("file evidence start line exceeds u32")?,
            end_line: u32::try_from(range.end()).context("file evidence end line exceeds u32")?,
            content_hash: snapshot.content_hash().as_str().to_string(),
        },
        EvidenceKind::PdfSpan {
            snapshot,
            page_start,
            page_end,
        } => EvidenceSourceResponse::Pdf {
            snapshot_id: snapshot.blob_id().value(),
            page_start: *page_start,
            page_end: *page_end,
        },
        EvidenceKind::PdfRegion {
            snapshot,
            page,
            x,
            y,
            width,
            height,
        } => EvidenceSourceResponse::PdfRegion {
            snapshot_id: snapshot.blob_id().value(),
            page: *page,
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
        EvidenceKind::WebSnapshot { url, snapshot, .. } => EvidenceSourceResponse::Web {
            url: url.clone(),
            content_hash: snapshot.content_hash().as_str().to_string(),
            snapshot_id: snapshot.blob_id().value(),
        },
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            blob,
        } => EvidenceSourceResponse::Command {
            harness_run: harness_run.value(),
            stream: format!("{stream:?}"),
            blob_id: blob.value(),
        },
        EvidenceKind::TestResult {
            harness_run,
            status,
            log,
        } => EvidenceSourceResponse::Test {
            harness_run: harness_run.value(),
            status: format!("{status:?}"),
            log_id: log.value(),
        },
        EvidenceKind::Diff {
            harness_run,
            patch_blob,
        } => EvidenceSourceResponse::Diff {
            harness_run: harness_run.value(),
            patch_blob_id: patch_blob.value(),
        },
        EvidenceKind::Validation { report_id } => EvidenceSourceResponse::Validation {
            report_id: report_id.value(),
        },
    })
}

fn task_summary(task: &Task) -> TaskSummary {
    TaskSummary {
        task_id: task.id.value(),
        title: task.title.clone(),
        status: format!("{:?}", task.status),
        priority: format!("{:?}", task.priority),
        evidence_ids: task.evidence_ids.iter().map(|id| id.value()).collect(),
        validation_report_id: task.status.validation_report_id().map(|id| id.value()),
    }
}

#[cfg(test)]
#[path = "read_services_tests.rs"]
mod tests;

pub(super) async fn dispatch_read(
    context: &ApiContext,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    match operation {
        ClientOperation::Status => {
            let layout = context.layout.clone();
            let socket_path = context.socket_path.clone();
            let response =
                support::run_database_retry("status", move || status(&layout, &socket_path))
                    .await?;
            Ok(ClientResponse::Status(response))
        }
        ClientOperation::Task { task_id } => {
            let layout = context.layout.clone();
            let response =
                support::run_database_retry("task", move || task(&layout, task_id)).await?;
            Ok(ClientResponse::Task(response))
        }
        ClientOperation::Evidence { evidence_id } => {
            let layout = context.layout.clone();
            let response = support::run_database_retry("evidence", move || {
                open_evidence(&layout, evidence_id)
            })
            .await?;
            Ok(ClientResponse::Evidence(response))
        }
        _ => Err(anyhow!("invalid read operation")),
    }
}
