use maestria_domain::ContentHash;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyFileSpan {
    kind: String,
    path: String,
    start: usize,
    end: usize,
    content_hash: String,
    #[serde(default)]
    snapshot: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPdfSpan {
    kind: String,
    blob: u64,
    page_start: u32,
    page_end: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPdfRegion {
    kind: String,
    blob: u64,
    page: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyWebSnapshot {
    kind: String,
    url: String,
    snapshot: u64,
    fetched_at: u64,
    content_hash: String,
    #[serde(default)]
    metadata: crate::payloads::web_evidence_payload::StoredWebEvidenceMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalSnapshot {
    blob_id: u64,
    content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalFileSpan {
    kind: String,
    #[serde(rename = "path")]
    _path: String,
    #[serde(rename = "start")]
    _start: usize,
    #[serde(rename = "end")]
    _end: usize,
    snapshot: CanonicalSnapshot,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalPdfSpan {
    kind: String,
    snapshot: CanonicalSnapshot,
    #[serde(rename = "page_start")]
    _page_start: u32,
    #[serde(rename = "page_end")]
    _page_end: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalPdfRegion {
    kind: String,
    snapshot: CanonicalSnapshot,
    #[serde(rename = "page")]
    _page: u32,
    #[serde(rename = "x")]
    _x: u32,
    #[serde(rename = "y")]
    _y: u32,
    #[serde(rename = "width")]
    _width: u32,
    #[serde(rename = "height")]
    _height: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanonicalWebSnapshot {
    kind: String,
    #[serde(rename = "url")]
    _url: String,
    snapshot: CanonicalSnapshot,
    #[serde(rename = "fetched_at")]
    _fetched_at: u64,
    #[serde(default, rename = "metadata")]
    _metadata: crate::payloads::web_evidence_payload::StoredWebEvidenceMetadata,
}

#[derive(Debug, Serialize)]
struct Snapshot<'a> {
    blob_id: u64,
    content_hash: &'a str,
}

#[derive(Debug, Serialize)]
struct MigratedFileSpan<'a> {
    kind: &'static str,
    path: String,
    start: usize,
    end: usize,
    snapshot: Snapshot<'a>,
}

#[derive(Debug, Serialize)]
struct MigratedPdfSpan<'a> {
    kind: &'static str,
    snapshot: Snapshot<'a>,
    page_start: u32,
    page_end: u32,
}

#[derive(Debug, Serialize)]
struct MigratedPdfRegion<'a> {
    kind: &'static str,
    snapshot: Snapshot<'a>,
    page: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Serialize)]
struct MigratedWebSnapshot<'a> {
    kind: &'static str,
    url: String,
    snapshot: Snapshot<'a>,
    fetched_at: u64,
    metadata: crate::payloads::web_evidence_payload::StoredWebEvidenceMetadata,
}

pub(super) fn migrate_kind_value(
    value: serde_json::Value,
    owner_hash: Option<&str>,
) -> Result<serde_json::Value, String> {
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "evidence kind is missing string kind".to_string())?;
    match kind {
        "file_span" => migrate_file_span(value),
        "pdf_span" => migrate_pdf_span(value, owner_hash),
        "pdf_region" => migrate_pdf_region(value, owner_hash),
        "web_snapshot" => migrate_web_snapshot(value),
        _ => Ok(value),
    }
}

fn migrate_file_span(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value
        .get("snapshot")
        .is_some_and(serde_json::Value::is_object)
    {
        let canonical: CanonicalFileSpan = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid canonical file_span: {error}"))?;
        if canonical.kind != "file_span" {
            return Err("file_span kind discriminator is inconsistent".to_string());
        }
        validate_snapshot(&canonical.snapshot)?;
        return Ok(value);
    }

    let legacy: LegacyFileSpan = serde_json::from_value(value)
        .map_err(|error| format!("invalid legacy file_span: {error}"))?;
    if legacy.kind != "file_span" {
        return Err("file_span kind discriminator is inconsistent".to_string());
    }
    let blob_id = legacy
        .snapshot
        .ok_or_else(|| "legacy file_span snapshot/blob identity is null".to_string())?;
    validate_snapshot_parts(blob_id, &legacy.content_hash)?;
    serde_json::to_value(MigratedFileSpan {
        kind: "file_span",
        path: legacy.path,
        start: legacy.start,
        end: legacy.end,
        snapshot: Snapshot {
            blob_id,
            content_hash: &legacy.content_hash,
        },
    })
    .map_err(|error| format!("serialize migrated file_span: {error}"))
}

fn migrate_pdf_span(
    value: serde_json::Value,
    owner_hash: Option<&str>,
) -> Result<serde_json::Value, String> {
    if value
        .get("snapshot")
        .is_some_and(serde_json::Value::is_object)
    {
        let canonical: CanonicalPdfSpan = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid canonical pdf_span: {error}"))?;
        if canonical.kind != "pdf_span" {
            return Err("pdf_span kind discriminator is inconsistent".to_string());
        }
        validate_snapshot(&canonical.snapshot)?;
        return Ok(value);
    }
    let owner_hash = owner_hash
        .ok_or_else(|| "pdf_span migration requires owning artifact content hash".to_string())?;
    let legacy: LegacyPdfSpan = serde_json::from_value(value)
        .map_err(|error| format!("invalid legacy pdf_span: {error}"))?;
    if legacy.kind != "pdf_span" {
        return Err("pdf_span kind discriminator is inconsistent".to_string());
    }
    serde_json::to_value(MigratedPdfSpan {
        kind: "pdf_span",
        snapshot: Snapshot {
            blob_id: legacy.blob,
            content_hash: owner_hash,
        },
        page_start: legacy.page_start,
        page_end: legacy.page_end,
    })
    .map_err(|error| format!("serialize migrated pdf_span: {error}"))
}

fn migrate_pdf_region(
    value: serde_json::Value,
    owner_hash: Option<&str>,
) -> Result<serde_json::Value, String> {
    if value
        .get("snapshot")
        .is_some_and(serde_json::Value::is_object)
    {
        let canonical: CanonicalPdfRegion = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid canonical pdf_region: {error}"))?;
        if canonical.kind != "pdf_region" {
            return Err("pdf_region kind discriminator is inconsistent".to_string());
        }
        validate_snapshot(&canonical.snapshot)?;
        return Ok(value);
    }
    let owner_hash = owner_hash
        .ok_or_else(|| "pdf_region migration requires owning artifact content hash".to_string())?;
    let legacy: LegacyPdfRegion = serde_json::from_value(value)
        .map_err(|error| format!("invalid legacy pdf_region: {error}"))?;
    if legacy.kind != "pdf_region" {
        return Err("pdf_region kind discriminator is inconsistent".to_string());
    }
    serde_json::to_value(MigratedPdfRegion {
        kind: "pdf_region",
        snapshot: Snapshot {
            blob_id: legacy.blob,
            content_hash: owner_hash,
        },
        page: legacy.page,
        x: legacy.x,
        y: legacy.y,
        width: legacy.width,
        height: legacy.height,
    })
    .map_err(|error| format!("serialize migrated pdf_region: {error}"))
}

fn migrate_web_snapshot(value: serde_json::Value) -> Result<serde_json::Value, String> {
    if value
        .get("snapshot")
        .is_some_and(serde_json::Value::is_object)
    {
        let canonical: CanonicalWebSnapshot = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid canonical web_snapshot: {error}"))?;
        if canonical.kind != "web_snapshot" {
            return Err("web_snapshot kind discriminator is inconsistent".to_string());
        }
        validate_snapshot(&canonical.snapshot)?;
        return Ok(value);
    }

    let legacy: LegacyWebSnapshot = serde_json::from_value(value)
        .map_err(|error| format!("invalid legacy web_snapshot: {error}"))?;
    if legacy.kind != "web_snapshot" {
        return Err("web_snapshot kind discriminator is inconsistent".to_string());
    }
    validate_snapshot_parts(legacy.snapshot, &legacy.content_hash)?;
    serde_json::to_value(MigratedWebSnapshot {
        kind: "web_snapshot",
        url: legacy.url,
        snapshot: Snapshot {
            blob_id: legacy.snapshot,
            content_hash: &legacy.content_hash,
        },
        fetched_at: legacy.fetched_at,
        metadata: legacy.metadata,
    })
    .map_err(|error| format!("serialize migrated web_snapshot: {error}"))
}

fn validate_snapshot(snapshot: &CanonicalSnapshot) -> Result<(), String> {
    validate_snapshot_parts(snapshot.blob_id, &snapshot.content_hash)
}

fn validate_snapshot_parts(_blob_id: u64, content_hash: &str) -> Result<(), String> {
    ContentHash::new(content_hash.to_owned())
        .map(|_| ())
        .map_err(|error| format!("invalid snapshot content hash: {error}"))
}
