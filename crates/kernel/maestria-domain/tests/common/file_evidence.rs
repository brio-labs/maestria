use maestria_domain::{BlobId, EvidenceKind, LineRange, SnapshotRef};
#[path = "content_hash.rs"]
mod fixtures;

pub fn file_span_kind() -> Result<EvidenceKind, Box<dyn std::error::Error>> {
    Ok(EvidenceKind::FileSpan {
        path: "notes.txt".to_string(),
        range: LineRange::new(2, 2)?,
        snapshot: SnapshotRef::new(BlobId::new(42), fixtures::test_content_hash()?),
    })
}
