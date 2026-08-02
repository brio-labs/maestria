use super::*;
use maestria_core::InstanceManifest;
use maestria_domain::{
    Artifact, ArtifactId, BlobId, ContentHash, Evidence, EvidenceId, EvidenceKind, IndexStatus,
    LineRange, ScopeId, SecurityMetadata, SnapshotRef, ValidationReportId,
};
use maestria_ports::{ArtifactRepository, EvidenceRepository};
use maestria_storage_sqlite::SqliteStore;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

impl TempDir {
    fn create() -> std::io::Result<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "maestria-services-test-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _temp_dir: TempDir,
    layout: InstanceLayout,
}

fn fixture() -> Result<Fixture> {
    let temp_dir = TempDir::create()?;
    let root = temp_dir.path().to_path_buf();
    let layout = InstanceLayout::for_root(root.clone());
    std::fs::create_dir_all(&layout.system_dir)?;
    if let Some(parent) = layout.database_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let manifest = InstanceManifest::default_for_root(root);
    std::fs::write(&layout.manifest_path, manifest.encode())?;

    Ok(Fixture {
        _temp_dir: temp_dir,
        layout,
    })
}

#[test]
fn open_evidence_rejects_file_span_outside_current_manifest_roots() -> Result<()> {
    let fixture = fixture()?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let artifact_id = ArtifactId::new(41);
    let evidence_id = EvidenceId::new(42);
    ArtifactRepository::put(
        &store,
        Artifact {
            id: artifact_id,
            title: "outside.md".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Indexed,
            content_hash: Some(ContentHash::new("sha256:".to_owned() + &"6".repeat(64))?),
            parse_status: None,
            security: maestria_domain::SecurityMetadata::default(),
        },
    )?;
    let outside_path = fixture.layout.root.join("..").join("indexed-outside.md");
    EvidenceRepository::put(
        &store,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::FileSpan {
                path: outside_path.display().to_string(),
                range: LineRange::new(1, 1)?,
                snapshot: SnapshotRef::new(
                    BlobId::new(1),
                    ContentHash::new(format!("sha256:{}", "0".repeat(64)))?,
                ),
            },
            excerpt: "outside".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: maestria_domain::SecurityMetadata::default(),
        },
    )?;
    drop(store);

    let error = match open_evidence(&fixture.layout, evidence_id.value()) {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "out-of-scope indexed evidence unexpectedly opened"
            ));
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("outside instance read roots or excluded by policy"),
        "unexpected out-of-scope evidence error: {error:#}"
    );
    Ok(())
}

#[test]
fn open_evidence_rejects_indexed_non_file_evidence_from_other_scope() -> Result<()> {
    let fixture = fixture()?;
    let store = SqliteStore::open(&fixture.layout.database_path)?;
    let artifact_id = ArtifactId::new(51);
    let evidence_id = EvidenceId::new(52);
    ArtifactRepository::put(
        &store,
        Artifact {
            id: artifact_id,
            title: "validation-report".to_string(),
            chunk_ids: BTreeSet::new(),
            card_ids: BTreeSet::new(),
            claim_ids: BTreeSet::new(),
            evidence_ids: BTreeSet::new(),
            index_status: IndexStatus::Indexed,
            content_hash: None,
            parse_status: None,
            security: SecurityMetadata {
                scope_id: Some(ScopeId::new(1)),
                ..SecurityMetadata::default()
            },
        },
    )?;
    EvidenceRepository::put(
        &store,
        Evidence {
            id: evidence_id,
            artifact_id,
            claim_id: None,
            kind: EvidenceKind::Validation {
                report_id: ValidationReportId::new(7),
            },
            excerpt: "validation report".to_string(),
            observed_at: maestria_domain::LogicalTick::new(1),
            security: SecurityMetadata {
                scope_id: Some(ScopeId::new(99)),
                ..SecurityMetadata::default()
            },
        },
    )?;
    drop(store);

    let error = match open_evidence(&fixture.layout, evidence_id.value()) {
        Ok(_) => {
            return Err(anyhow::anyhow!(
                "cross-instance indexed non-file evidence unexpectedly opened"
            ));
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("evidence is not available under retrieval policy: Scope mismatch"),
        "unexpected cross-instance evidence error: {error:#}"
    );
    Ok(())
}
