use maestria_domain::{
    BlobId, ClaimStatus, ContentRange, EvidenceKind, HarnessRunId, LogicalTick, OutputStream,
    TaskPriority, TaskStatus, ValidationReportId,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredEvidenceKind {
    FileSpan {
        path: String,
        start: usize,
        end: usize,
        content_hash: String,
    },
    PdfSpan {
        blob: u64,
        page_start: u32,
        page_end: u32,
    },
    WebSnapshot {
        url: String,
        snapshot: u64,
        fetched_at: u64,
        content_hash: String,
    },
    CommandOutput {
        harness_run: u64,
        stream: StoredOutputStream,
        blob: u64,
    },
    TestResult {
        harness_run: u64,
        status: StoredTestStatus,
        log: u64,
    },
    Diff {
        harness_run: u64,
        patch_blob: u64,
    },
    Validation {
        report_id: u64,
    },
}

impl StoredEvidenceKind {
    pub(crate) fn from_domain(kind: &EvidenceKind) -> Self {
        match kind {
            EvidenceKind::FileSpan {
                path,
                range,
                content_hash,
            } => Self::FileSpan {
                path: path.clone(),
                start: range.start,
                end: range.end,
                content_hash: content_hash.clone(),
            },
            EvidenceKind::PdfSpan {
                blob,
                page_start,
                page_end,
            } => Self::PdfSpan {
                blob: blob.value(),
                page_start: *page_start,
                page_end: *page_end,
            },
            EvidenceKind::WebSnapshot {
                url,
                snapshot,
                fetched_at,
                content_hash,
            } => Self::WebSnapshot {
                url: url.clone(),
                snapshot: snapshot.value(),
                fetched_at: fetched_at.value(),
                content_hash: content_hash.clone(),
            },
            EvidenceKind::CommandOutput {
                harness_run,
                stream,
                blob,
            } => Self::CommandOutput {
                harness_run: harness_run.value(),
                stream: StoredOutputStream::from_domain(*stream),
                blob: blob.value(),
            },
            EvidenceKind::TestResult {
                harness_run,
                status,
                log,
            } => Self::TestResult {
                harness_run: harness_run.value(),
                status: StoredTestStatus::from_domain(*status),
                log: log.value(),
            },
            EvidenceKind::Diff {
                harness_run,
                patch_blob,
            } => Self::Diff {
                harness_run: harness_run.value(),
                patch_blob: patch_blob.value(),
            },
            EvidenceKind::Validation { report_id } => Self::Validation {
                report_id: report_id.value(),
            },
        }
    }

    pub(crate) fn into_domain(self) -> EvidenceKind {
        match self {
            Self::FileSpan {
                path,
                start,
                end,
                content_hash,
            } => EvidenceKind::FileSpan {
                path,
                range: ContentRange { start, end },
                content_hash,
            },
            Self::PdfSpan {
                blob,
                page_start,
                page_end,
            } => EvidenceKind::PdfSpan {
                blob: BlobId::new(blob),
                page_start,
                page_end,
            },
            Self::WebSnapshot {
                url,
                snapshot,
                fetched_at,
                content_hash,
            } => EvidenceKind::WebSnapshot {
                url,
                snapshot: BlobId::new(snapshot),
                fetched_at: LogicalTick::new(fetched_at),
                content_hash,
            },
            Self::CommandOutput {
                harness_run,
                stream,
                blob,
            } => EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(harness_run),
                stream: stream.into_domain(),
                blob: BlobId::new(blob),
            },
            Self::TestResult {
                harness_run,
                status,
                log,
            } => EvidenceKind::TestResult {
                harness_run: HarnessRunId::new(harness_run),
                status: status.into_domain(),
                log: BlobId::new(log),
            },
            Self::Diff {
                harness_run,
                patch_blob,
            } => EvidenceKind::Diff {
                harness_run: HarnessRunId::new(harness_run),
                patch_blob: BlobId::new(patch_blob),
            },
            Self::Validation { report_id } => EvidenceKind::Validation {
                report_id: ValidationReportId::new(report_id),
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredOutputStream {
    Stdout,
    Stderr,
    Combined,
}

impl StoredOutputStream {
    pub(crate) fn from_domain(stream: OutputStream) -> Self {
        match stream {
            OutputStream::Stdout => Self::Stdout,
            OutputStream::Stderr => Self::Stderr,
            OutputStream::Combined => Self::Combined,
        }
    }

    pub(crate) fn into_domain(self) -> OutputStream {
        match self {
            Self::Stdout => OutputStream::Stdout,
            Self::Stderr => OutputStream::Stderr,
            Self::Combined => OutputStream::Combined,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredTestStatus {
    Passed,
    Failed,
    TimedOut,
}

impl StoredTestStatus {
    pub(crate) fn from_domain(status: maestria_domain::TestStatus) -> Self {
        match status {
            maestria_domain::TestStatus::Passed => Self::Passed,
            maestria_domain::TestStatus::Failed => Self::Failed,
            maestria_domain::TestStatus::TimedOut => Self::TimedOut,
        }
    }

    pub(crate) fn into_domain(self) -> maestria_domain::TestStatus {
        match self {
            Self::Passed => maestria_domain::TestStatus::Passed,
            Self::Failed => maestria_domain::TestStatus::Failed,
            Self::TimedOut => maestria_domain::TestStatus::TimedOut,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredTaskPriority {
    Low,
    Normal,
    High,
}

impl StoredTaskPriority {
    pub(crate) fn from_domain(priority: TaskPriority) -> Self {
        match priority {
            TaskPriority::Low => Self::Low,
            TaskPriority::Normal => Self::Normal,
            TaskPriority::High => Self::High,
        }
    }

    pub(crate) fn into_domain(self) -> TaskPriority {
        match self {
            Self::Low => TaskPriority::Low,
            Self::Normal => TaskPriority::Normal,
            Self::High => TaskPriority::High,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredTaskStatus {
    Draft,
    Open,
    Active,
    Validating,
    Blocked,
    CompletedVerified,
    CompletedWithWarnings,
    Failed,
    Cancelled,
}

impl StoredTaskStatus {
    pub(crate) fn from_domain(status: TaskStatus) -> Self {
        match status {
            TaskStatus::Draft => Self::Draft,
            TaskStatus::Open => Self::Open,
            TaskStatus::Active => Self::Active,
            TaskStatus::Validating => Self::Validating,
            TaskStatus::Blocked => Self::Blocked,
            TaskStatus::CompletedVerified => Self::CompletedVerified,
            TaskStatus::CompletedWithWarnings => Self::CompletedWithWarnings,
            TaskStatus::Failed => Self::Failed,
            TaskStatus::Cancelled => Self::Cancelled,
        }
    }

    pub(crate) fn into_domain(self) -> TaskStatus {
        match self {
            Self::Draft => TaskStatus::Draft,
            Self::Open => TaskStatus::Open,
            Self::Active => TaskStatus::Active,
            Self::Validating => TaskStatus::Validating,
            Self::Blocked => TaskStatus::Blocked,
            Self::CompletedVerified => TaskStatus::CompletedVerified,
            Self::CompletedWithWarnings => TaskStatus::CompletedWithWarnings,
            Self::Failed => TaskStatus::Failed,
            Self::Cancelled => TaskStatus::Cancelled,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StoredClaimStatus {
    Draft,
    Proposed,
    Verified,
    Disputed,
    Archived,
}

impl StoredClaimStatus {
    pub(crate) fn from_domain(status: &ClaimStatus) -> Self {
        match status {
            ClaimStatus::Draft => Self::Draft,
            ClaimStatus::Proposed => Self::Proposed,
            ClaimStatus::Verified => Self::Verified,
            ClaimStatus::Disputed => Self::Disputed,
            ClaimStatus::Archived => Self::Archived,
        }
    }

    pub(crate) fn into_domain(self) -> ClaimStatus {
        match self {
            Self::Draft => ClaimStatus::Draft,
            Self::Proposed => ClaimStatus::Proposed,
            Self::Verified => ClaimStatus::Verified,
            Self::Disputed => ClaimStatus::Disputed,
            Self::Archived => ClaimStatus::Archived,
        }
    }
}
