use std::fs;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use maestria_blob_fs::FsBlobStore;
use maestria_core::{CorePorts, CoreServices, InstanceLayout, InstanceManifest, OpenEvidenceInput};
use maestria_domain::{Evidence, EvidenceKind, KernelState, ScopeId, Task};
use maestria_governance::PrivacyExclusions;
use maestria_parsers::ParserRegistry;
use maestria_ports::{ArtifactRepository, EvidenceRepository};
use maestria_search_tantivy::TantivyFullTextIndex;
use maestria_storage_sqlite::SqliteStore;

use super::super::protocol::{
    EvidenceResponse, EvidenceSourceResponse, StatusResponse, TaskResponse, TaskSummary,
};

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

pub(super) async fn run_database_retry<T, F>(operation_name: &str, operation: F) -> Result<T>
where
    T: Send + 'static,
    F: Fn() -> Result<T> + Send + Sync + 'static,
{
    let operation = Arc::new(operation);
    for attempt in 0..super::DATABASE_RETRY_ATTEMPTS {
        let op = Arc::clone(&operation);
        let result = tokio::task::spawn_blocking(move || op())
            .await
            .map_err(|error| anyhow!("{operation_name} task failed: {error}"))?;
        match result {
            Ok(response) => return Ok(response),
            Err(error)
                if super::is_database_locked(&error)
                    && attempt + 1 < super::DATABASE_RETRY_ATTEMPTS =>
            {
                tokio::time::sleep(super::DATABASE_RETRY_DELAY).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(anyhow!("{operation_name} retries exhausted"))
}

pub(super) fn open_evidence(layout: &InstanceLayout, evidence_id: u64) -> Result<EvidenceResponse> {
    let manifest = InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    let sqlite = SqliteStore::open(&layout.database_path)?;
    let evidence_id = maestria_domain::EvidenceId::new(evidence_id);
    let retrieval_policy = maestria_governance::RetrievalSecurityPolicy::default()
        .require_read_allowed(true)
        .required_scope(ScopeId::new(1))
        .allow_unscoped_items(true);
    if let Some(evidence) = EvidenceRepository::get(&sqlite, evidence_id)? {
        if let maestria_governance::RetrievalDecision::Denied(reason) =
            retrieval_policy.evaluate(&evidence.security)
        {
            return Err(anyhow!(
                "evidence is not available under retrieval policy: {reason}"
            ));
        }
        validate_evidence_scope(&manifest, &evidence)?;
        if let Some(artifact) = ArtifactRepository::get(&sqlite, evidence.artifact_id)?
            && let maestria_governance::RetrievalDecision::Denied(reason) =
                retrieval_policy.evaluate(&artifact.security)
        {
            return Err(anyhow!(
                "artifact is not available under retrieval policy: {reason}"
            ));
        }
    }
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
    })
    .with_retrieval_policy(retrieval_policy);
    let output = core.open_evidence(OpenEvidenceInput { evidence_id })?;
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

fn validate_evidence_scope(manifest: &InstanceManifest, evidence: &Evidence) -> Result<()> {
    let EvidenceKind::FileSpan { path, .. } = &evidence.kind else {
        return Ok(());
    };
    if source_scope_allowed(manifest, path) {
        return Ok(());
    }
    Err(anyhow!(
        "evidence source path {path} is outside instance read roots or excluded by policy"
    ))
}

fn source_scope_allowed(manifest: &InstanceManifest, path: &str) -> bool {
    let path = std::path::Path::new(path);
    let mut candidates = vec![lexical_normalize(path)];
    if path.is_relative() {
        candidates.push(lexical_normalize(&manifest.root.join(path)));
    }
    let roots: Vec<_> = manifest
        .read_roots
        .iter()
        .map(|root| lexical_normalize(root))
        .collect();
    let blocked_patterns = runtime_blocked_patterns(manifest);
    candidates.iter().any(|candidate| {
        roots.iter().any(|root| candidate.starts_with(root))
            && !blocked_patterns
                .iter()
                .any(|pattern| path_matches_pattern(candidate, pattern))
    })
}

fn runtime_blocked_patterns(manifest: &InstanceManifest) -> Vec<String> {
    let default_privacy = PrivacyExclusions::default();
    let mut blocked_patterns = manifest.excluded_patterns.clone();
    blocked_patterns.extend(default_privacy.sensitive_names().iter().cloned());
    blocked_patterns.extend(
        default_privacy
            .sensitive_extensions()
            .iter()
            .map(|extension| format!("*.{extension}")),
    );
    blocked_patterns
}

fn lexical_normalize(path: &std::path::Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn path_matches_pattern(path: &std::path::Path, pattern: &str) -> bool {
    path.components()
        .any(|component| glob_matches(&component.as_os_str().to_string_lossy(), pattern))
}

fn glob_matches(value: &str, pattern: &str) -> bool {
    let value: Vec<char> = value.chars().collect();
    let pattern: Vec<char> = pattern.chars().collect();
    let mut value_index = 0usize;
    let mut pattern_index = 0usize;
    let mut star_pattern_index = None;
    let mut star_value_index = 0usize;

    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == '?' || pattern[pattern_index] == value[value_index])
        {
            value_index += 1;
            pattern_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == '*' {
            star_pattern_index = Some(pattern_index);
            star_value_index = value_index;
            pattern_index += 1;
        } else if let Some(star_index) = star_pattern_index {
            star_value_index += 1;
            value_index = star_value_index;
            pattern_index = star_index + 1;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == '*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

pub(super) fn load_state_and_manifest(
    layout: &InstanceLayout,
) -> Result<(KernelState, InstanceManifest)> {
    let state = load_state(layout)?;
    let manifest = InstanceManifest::decode(&fs::read_to_string(&layout.manifest_path)?)
        .map_err(|error| anyhow!("parse instance manifest: {error}"))?;
    Ok((state, manifest))
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
        validation_report_id: task.validation_report_id.map(|id| id.value()),
    }
}

#[cfg(test)]
#[path = "read_services_tests.rs"]
mod tests;
