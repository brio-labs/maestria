use std::fs;

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceLayout, InstanceManifest, OpenEvidenceInput};
use maestria_domain::{
    Evidence, EvidenceCandidate, EvidenceKind, EvidenceSpan, KernelState, SearchOutcome, Task,
};
use maestria_parsers::ParserRegistry;
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

use super::server::ApiContext;
use super::{
    ClientOperation, ClientResponse, CoverageResponse, EvidenceResponse, EvidenceSourceResponse,
    SearchEvidenceResponse, SearchResponse, StatusResponse, TaskResponse, TaskSummary,
};

const MAX_SEARCH_LIMIT: usize = 100;

pub(crate) async fn dispatch(
    context: &ApiContext,
    operation: ClientOperation,
) -> Result<ClientResponse> {
    match operation {
        ClientOperation::Status => {
            let layout = context.layout.clone();
            let socket_path = context.socket_path.clone();
            let response = tokio::task::spawn_blocking(move || status(&layout, &socket_path))
                .await
                .map_err(|error| anyhow!("status task failed: {error}"))??;
            Ok(ClientResponse::Status(response))
        }
        ClientOperation::Task { task_id } => {
            let layout = context.layout.clone();
            let response = tokio::task::spawn_blocking(move || task(&layout, task_id))
                .await
                .map_err(|error| anyhow!("task query failed: {error}"))??;
            Ok(ClientResponse::Task(response))
        }
        ClientOperation::Evidence { evidence_id } => {
            let layout = context.layout.clone();
            let response = tokio::task::spawn_blocking(move || open_evidence(&layout, evidence_id))
                .await
                .map_err(|error| anyhow!("evidence query failed: {error}"))??;
            Ok(ClientResponse::Evidence(response))
        }
        ClientOperation::Search { query, limit } => {
            if query.trim().is_empty() {
                return Err(anyhow!("search query must not be empty"));
            }
            if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
                return Err(anyhow!(
                    "search limit must be between 1 and {MAX_SEARCH_LIMIT}"
                ));
            }
            Ok(ClientResponse::Search(search(context, query, limit).await?))
        }
    }
}

fn status(layout: &InstanceLayout, socket_path: &std::path::Path) -> Result<StatusResponse> {
    let state = load_state(layout)?;
    Ok(StatusResponse {
        instance_root: layout.root.display().to_string(),
        event_count: state.event_log.len(),
        task_count: state.tasks.len(),
        socket_path: socket_path.display().to_string(),
    })
}

fn task(layout: &InstanceLayout, task_id: Option<u64>) -> Result<TaskResponse> {
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

async fn search(context: &ApiContext, query: String, limit: usize) -> Result<SearchResponse> {
    let layout = context.layout.clone();
    let (state, manifest) = tokio::task::spawn_blocking(move || load_state_and_manifest(&layout))
        .await
        .map_err(|error| anyhow!("load search state task failed: {error}"))??;
    let layout = context.layout.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        crate::prepare_search_runtime_read_only(
            &layout,
            &state,
            &manifest,
            maestria_governance::RetrievalSecurityPolicy::default()
                .require_read_allowed(true)
                .allow_unscoped_items(true),
        )
    })
    .await
    .map_err(|error| anyhow!("prepare search runtime task failed: {error}"))??;
    let (plan, outcome) = runtime.execute(query, limit).await?;
    Ok(search_response(
        plan.original_query,
        plan.query_id.value(),
        outcome,
    ))
}

fn open_evidence(layout: &InstanceLayout, evidence_id: u64) -> Result<EvidenceResponse> {
    let sqlite = SqliteStore::open(&layout.database_path)?;
    let blobs = FsBlobStore::open(&layout.blobs_dir)?;
    let search_index = TantivyFullTextIndex::open_read_only(&layout.full_text_index_dir)?;
    let parser = ParserRegistry::with_defaults();
    let core = CoreServices::new(CorePorts {
        artifacts: &sqlite,
        chunks: &sqlite,
        cards: &sqlite,
        evidence: &sqlite,
        events: &sqlite,
        parser: &parser,
        search_index: &search_index,
        blobs: &blobs,
        vector_index: None,
        graph_index: None,
    });
    let output = core.open_evidence(OpenEvidenceInput {
        evidence_id: maestria_domain::EvidenceId::new(evidence_id),
    })?;
    Ok(EvidenceResponse {
        evidence_id: output.evidence.id.value(),
        artifact_id: output.artifact.id.value(),
        artifact_title: output.artifact.title,
        artifact_content_hash: output.artifact.content_hash,
        source: evidence_source(&output.evidence)?,
        excerpt: output.evidence.excerpt,
        observed_at: output.evidence.observed_at.value(),
    })
}

fn load_state(layout: &InstanceLayout) -> Result<KernelState> {
    crate::load_kernel_state(layout)
}

fn load_state_and_manifest(layout: &InstanceLayout) -> Result<(KernelState, InstanceManifest)> {
    let state = load_state(layout)?;
    let manifest = InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    Ok((state, manifest))
}

fn search_response(query: String, query_id: u64, outcome: SearchOutcome) -> SearchResponse {
    SearchResponse {
        query,
        query_id,
        trace_id: outcome.trace.value(),
        status: format!("{:?}", outcome.status),
        fingerprint: outcome.fingerprint.as_str().to_string(),
        index_generation: outcome.index_generation.value(),
        evidence: outcome.evidence.iter().map(search_evidence).collect(),
        coverage: CoverageResponse {
            percent_covered: outcome.coverage.percent_covered,
            gaps: outcome.coverage.gaps_identified,
            distinct_sources: outcome.coverage.distinct_sources,
            distinct_documents: outcome.coverage.distinct_documents,
            distinct_sections: outcome.coverage.distinct_sections,
        },
        conflict_count: outcome.conflicts.len(),
    }
}

fn search_evidence(candidate: &EvidenceCandidate) -> SearchEvidenceResponse {
    SearchEvidenceResponse {
        evidence_id: candidate.evidence_id.value(),
        artifact_version: candidate.artifact_version.value(),
        source: format_source_span(&candidate.source_span),
        range_start: candidate.source_span.range().start,
        range_end: candidate.source_span.range().end,
        lexical_score: candidate.scores.bm25,
        semantic_score: candidate.scores.semantic_similarity,
        trust: format!("{:?}", candidate.trust),
        freshness: format!("{:?}", candidate.freshness),
    }
}

fn format_source_span(span: &EvidenceSpan) -> String {
    match span.location() {
        maestria_domain::SourceLocation::File {
            path,
            start_line,
            end_line,
        } => {
            format!("{path}:{start_line}-{end_line}")
        }
        maestria_domain::SourceLocation::Page {
            page_start,
            page_end,
        } => {
            format!("pages {page_start}-{page_end}")
        }
        maestria_domain::SourceLocation::Region {
            page,
            x,
            y,
            width,
            height,
        } => {
            format!("page {page} region {x},{y} {width}x{height}")
        }
        maestria_domain::SourceLocation::Symbol {
            path,
            qualified_name,
        } => {
            format!("{path}::{qualified_name}")
        }
    }
}

fn evidence_source(evidence: &Evidence) -> Result<EvidenceSourceResponse> {
    Ok(match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            content_hash,
            ..
        } => EvidenceSourceResponse::File {
            path: path.clone(),
            start_line: u32::try_from(range.start)
                .context("file evidence start line exceeds u32")?,
            end_line: u32::try_from(range.end).context("file evidence end line exceeds u32")?,
            content_hash: content_hash.clone(),
        },
        EvidenceKind::PdfSpan {
            blob,
            page_start,
            page_end,
        } => EvidenceSourceResponse::Pdf {
            snapshot_id: blob.value(),
            page_start: *page_start,
            page_end: *page_end,
        },
        EvidenceKind::PdfRegion {
            blob,
            page,
            x,
            y,
            width,
            height,
        } => EvidenceSourceResponse::PdfRegion {
            snapshot_id: blob.value(),
            page: *page,
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
        EvidenceKind::WebSnapshot {
            url,
            snapshot,
            content_hash,
            ..
        } => EvidenceSourceResponse::Web {
            url: url.clone(),
            content_hash: content_hash.clone(),
            snapshot_id: snapshot.value(),
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
        validation_report_id: task.validation_report_id.map(|id| id.value()),
    }
}
