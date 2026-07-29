use super::web_evidence_payload::StoredWebEvidenceMetadata;
use maestria_domain::{
    BlobId, ClaimStatus, ContentHash, EvidenceKind, HarnessRunId, LineRange, LogicalTick,
    OutputStream, SnapshotRef, TaskPriority, TaskStatus, ValidationReportId,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredSnapshotRef {
    blob_id: u64,
    content_hash: String,
}

impl From<&SnapshotRef> for StoredSnapshotRef {
    fn from(snapshot: &SnapshotRef) -> Self {
        Self {
            blob_id: snapshot.blob_id().value(),
            content_hash: snapshot.content_hash().as_str().to_owned(),
        }
    }
}

impl TryFrom<StoredSnapshotRef> for SnapshotRef {
    type Error = String;

    fn try_from(snapshot: StoredSnapshotRef) -> Result<Self, Self::Error> {
        let content_hash = ContentHash::new(snapshot.content_hash)
            .map_err(|error| format!("invalid snapshot content hash: {error}"))?;
        Ok(Self::new(BlobId::new(snapshot.blob_id), content_hash))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StoredEvidenceKind {
    FileSpan {
        path: String,
        start: usize,
        end: usize,
        snapshot: StoredSnapshotRef,
    },
    PdfSpan {
        snapshot: StoredSnapshotRef,
        page_start: u32,
        page_end: u32,
    },
    PdfRegion {
        snapshot: StoredSnapshotRef,
        page: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    WebSnapshot {
        url: String,
        snapshot: StoredSnapshotRef,
        fetched_at: u64,
        #[serde(default)]
        metadata: StoredWebEvidenceMetadata,
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
                snapshot,
            } => Self::FileSpan {
                path: path.clone(),
                start: range.start(),
                end: range.end(),
                snapshot: StoredSnapshotRef::from(snapshot),
            },
            EvidenceKind::PdfSpan {
                snapshot,
                page_start,
                page_end,
            } => Self::PdfSpan {
                snapshot: StoredSnapshotRef::from(snapshot),
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
            } => Self::PdfRegion {
                snapshot: StoredSnapshotRef::from(snapshot),
                page: *page,
                x: *x,
                y: *y,
                width: *width,
                height: *height,
            },
            EvidenceKind::WebSnapshot {
                url,
                snapshot,
                fetched_at,
                metadata,
            } => Self::WebSnapshot {
                url: url.clone(),
                snapshot: StoredSnapshotRef::from(snapshot),
                fetched_at: fetched_at.value(),
                metadata: StoredWebEvidenceMetadata::from_domain(metadata),
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

    pub(crate) fn try_into_domain(self) -> Result<EvidenceKind, String> {
        self.try_into()
    }
}

impl TryFrom<StoredEvidenceKind> for EvidenceKind {
    type Error = String;

    fn try_from(value: StoredEvidenceKind) -> Result<Self, Self::Error> {
        match value {
            StoredEvidenceKind::FileSpan {
                path,
                start,
                end,
                snapshot,
            } => {
                let snapshot = SnapshotRef::try_from(snapshot)?;
                Ok(EvidenceKind::FileSpan {
                    path,
                    range: LineRange::new(start, end)
                        .map_err(|error| format!("invalid evidence line range: {error}"))?,
                    snapshot,
                })
            }
            StoredEvidenceKind::PdfRegion {
                snapshot,
                page,
                x,
                y,
                width,
                height,
            } => Ok(EvidenceKind::PdfRegion {
                snapshot: SnapshotRef::try_from(snapshot)?,
                page,
                x,
                y,
                width,
                height,
            }),
            StoredEvidenceKind::PdfSpan {
                snapshot,
                page_start,
                page_end,
            } => Ok(EvidenceKind::PdfSpan {
                snapshot: SnapshotRef::try_from(snapshot)?,
                page_start,
                page_end,
            }),
            StoredEvidenceKind::WebSnapshot {
                url,
                snapshot,
                fetched_at,
                metadata,
            } => Ok(EvidenceKind::WebSnapshot {
                url,
                snapshot: SnapshotRef::try_from(snapshot)?,
                fetched_at: LogicalTick::new(fetched_at),
                metadata: metadata.into_domain(),
            }),
            StoredEvidenceKind::CommandOutput {
                harness_run,
                stream,
                blob,
            } => Ok(EvidenceKind::CommandOutput {
                harness_run: HarnessRunId::new(harness_run),
                stream: stream.into_domain(),
                blob: BlobId::new(blob),
            }),
            StoredEvidenceKind::TestResult {
                harness_run,
                status,
                log,
            } => Ok(EvidenceKind::TestResult {
                harness_run: HarnessRunId::new(harness_run),
                status: status.into_domain(),
                log: BlobId::new(log),
            }),
            StoredEvidenceKind::Diff {
                harness_run,
                patch_blob,
            } => Ok(EvidenceKind::Diff {
                harness_run: HarnessRunId::new(harness_run),
                patch_blob: BlobId::new(patch_blob),
            }),
            StoredEvidenceKind::Validation { report_id } => Ok(EvidenceKind::Validation {
                report_id: ValidationReportId::new(report_id),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[cfg(test)]
#[path = "evidence_payload_tests.rs"]
mod tests;
