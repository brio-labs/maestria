use crate::error::{CoreError, CoreResult};
use crate::ports::CorePorts;
use maestria_domain::{Evidence, EvidenceKind};

use crate::provenance::content_hash;

pub(super) fn verify_source_snapshot(ports: &CorePorts<'_>, evidence: &Evidence) -> CoreResult<()> {
    if let EvidenceKind::WebSnapshot {
        snapshot,
        content_hash: expected_hash,
        ..
    } = &evidence.kind
    {
        let bytes = ports.blobs.get(*snapshot)?;
        let actual_hash = content_hash(&bytes);
        if &actual_hash != expected_hash {
            return Err(CoreError::InvalidInput {
                message: format!(
                    "evidence {} web snapshot hash mismatch: expected {expected_hash}, got {actual_hash}",
                    evidence.id
                ),
            });
        }
        let source = String::from_utf8_lossy(&bytes);
        if !evidence.excerpt.is_empty() && !source.contains(&evidence.excerpt) {
            return Err(CoreError::InvalidInput {
                message: format!(
                    "evidence {} excerpt is absent from its web snapshot",
                    evidence.id
                ),
            });
        }
        return Ok(());
    }

    let Evidence {
        kind:
            EvidenceKind::FileSpan {
                range,
                content_hash: expected_hash,
                snapshot: Some(snapshot),
                ..
            },
        excerpt,
        ..
    } = evidence
    else {
        return Ok(());
    };

    let bytes = ports.blobs.get(*snapshot)?;
    let actual_hash = content_hash(&bytes);
    if &actual_hash != expected_hash {
        return Err(CoreError::InvalidInput {
            message: format!(
                "evidence {} snapshot hash mismatch: expected {expected_hash}, got {actual_hash}",
                evidence.id
            ),
        });
    }

    let source = String::from_utf8_lossy(&bytes);
    let line_count = source.lines().count().max(1);
    if range.start == 0 || range.end < range.start || range.end > line_count {
        return Err(CoreError::InvalidInput {
            message: format!("evidence {} has an invalid source line range", evidence.id),
        });
    }
    let compact_source = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if !excerpt.is_empty() && !compact_source.contains(excerpt) {
        return Err(CoreError::InvalidInput {
            message: format!(
                "evidence {} excerpt is absent from its source snapshot",
                evidence.id
            ),
        });
    }
    Ok(())
}
