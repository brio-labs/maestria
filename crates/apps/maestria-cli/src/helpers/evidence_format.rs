pub(crate) fn source_label(evidence: &maestria_domain::Evidence) -> String {
    use maestria_domain::EvidenceKind;

    match &evidence.kind {
        EvidenceKind::FileSpan {
            path,
            range,
            snapshot,
        } => format!(
            "source=file path={} lines={}-{} hash={}",
            path,
            range.start(),
            range.end(),
            snapshot.content_hash().as_str()
        ),
        EvidenceKind::PdfSpan {
            snapshot,
            page_start,
            page_end,
        } => format!(
            "source=pdf blob={} pages={}-{} hash={}",
            snapshot.blob_id(),
            page_start,
            page_end,
            snapshot.content_hash().as_str()
        ),
        EvidenceKind::PdfRegion {
            snapshot,
            page,
            x,
            y,
            width,
            height,
        } => format!(
            "source=pdf blob={} page={} region={},{} {}x{} hash={}",
            snapshot.blob_id(),
            page,
            x,
            y,
            width,
            height,
            snapshot.content_hash().as_str()
        ),
        EvidenceKind::WebSnapshot { url, snapshot, .. } => {
            format!("source=web url={} snapshot={}", url, snapshot.blob_id())
        }
        EvidenceKind::CommandOutput {
            harness_run,
            stream,
            blob,
        } => format!(
            "source=command run={} stream={:?} blob={}",
            harness_run, stream, blob
        ),
        EvidenceKind::TestResult {
            harness_run,
            status,
            log,
        } => format!(
            "source=test run={} status={:?} log={}",
            harness_run, status, log
        ),
        EvidenceKind::Diff {
            harness_run,
            patch_blob,
        } => format!("source=diff run={} patch={}", harness_run, patch_blob),
        EvidenceKind::Validation { report_id } => {
            format!("source=validation report={}", report_id)
        }
    }
}
