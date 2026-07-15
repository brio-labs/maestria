use maestria_domain::{
    ArtifactId, CardId, ClaimId, EvidenceId, MemoryId, Relation, RelationEndpoint, RelationId,
    RelationKind, SecurityMetadata, TaskId,
};
use maestria_ports::PortError;
use rusqlite::Row;

pub(super) fn read_relation(row: &Row<'_>) -> Result<Relation, PortError> {
    let id = row.get::<_, String>(0).map_err(to_port_error)?;
    let source_type = row.get::<_, String>(1).map_err(to_port_error)?;
    let source_id = row.get::<_, String>(2).map_err(to_port_error)?;
    let kind = row.get::<_, String>(3).map_err(to_port_error)?;
    let target_type = row.get::<_, String>(4).map_err(to_port_error)?;
    let target_id = row.get::<_, String>(5).map_err(to_port_error)?;
    let evidence_id = row.get::<_, Option<String>>(6).map_err(to_port_error)?;
    let confidence_milli = row.get::<_, i64>(7).map_err(to_port_error)?;
    let security_json = row.get::<_, String>(8).map_err(to_port_error)?;
    let security: SecurityMetadata =
        serde_json::from_str(&security_json).map_err(|error| PortError::Internal {
            message: format!("deserialize relation security: {error}"),
        })?;

    Ok(Relation {
        id: parse_relation_id(&id)?,
        source: relation_endpoint_from_parts(&source_type, &source_id)?,
        kind: relation_kind_from_str(&kind)?,
        target: relation_endpoint_from_parts(&target_type, &target_id)?,
        evidence_id: evidence_id.as_deref().map(parse_evidence_id).transpose()?,
        confidence_milli: i64_to_u16(confidence_milli, "confidence_milli")?,
        security,
    })
}

pub(super) fn relation_endpoint_to_parts(endpoint: RelationEndpoint) -> (&'static str, String) {
    match endpoint {
        RelationEndpoint::Artifact(id) => ("artifact", id.value().to_string()),
        RelationEndpoint::Claim(id) => ("claim", id.value().to_string()),
        RelationEndpoint::Task(id) => ("task", id.value().to_string()),
        RelationEndpoint::Memory(id) => ("memory", id.value().to_string()),
        RelationEndpoint::Card(id) => ("card", id.value().to_string()),
    }
}

pub(super) fn relation_endpoint_from_parts(
    endpoint_type: &str,
    endpoint_id: &str,
) -> Result<RelationEndpoint, PortError> {
    match endpoint_type {
        "artifact" => Ok(RelationEndpoint::Artifact(ArtifactId::new(parse_u64(
            endpoint_id,
            "artifact id",
        )?))),
        "claim" => Ok(RelationEndpoint::Claim(ClaimId::new(parse_u64(
            endpoint_id,
            "claim id",
        )?))),
        "task" => Ok(RelationEndpoint::Task(TaskId::new(parse_u64(
            endpoint_id,
            "task id",
        )?))),
        "memory" => Ok(RelationEndpoint::Memory(MemoryId::new(parse_u64(
            endpoint_id,
            "memory id",
        )?))),
        "card" => Ok(RelationEndpoint::Card(CardId::new(parse_u64(
            endpoint_id,
            "card id",
        )?))),
        other => Err(PortError::Internal {
            message: format!("unknown relation endpoint type {other}"),
        }),
    }
}

pub(super) fn relation_kind_to_str(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Contains => "contains",
        RelationKind::Defines => "defines",
        RelationKind::Supports => "supports",
        RelationKind::Contradicts => "contradicts",
        RelationKind::UsedEvidence => "used_evidence",
        RelationKind::BasedOn => "based_on",
        RelationKind::DerivedFrom => "derived_from",
        RelationKind::AppliesTo => "applies_to",
        RelationKind::RelatedTo => "related_to",
    }
}

pub(super) fn relation_kind_from_str(kind: &str) -> Result<RelationKind, PortError> {
    match kind {
        "contains" => Ok(RelationKind::Contains),
        "defines" => Ok(RelationKind::Defines),
        "supports" => Ok(RelationKind::Supports),
        "contradicts" => Ok(RelationKind::Contradicts),
        "used_evidence" => Ok(RelationKind::UsedEvidence),
        "based_on" => Ok(RelationKind::BasedOn),
        "derived_from" => Ok(RelationKind::DerivedFrom),
        "applies_to" => Ok(RelationKind::AppliesTo),
        "related_to" => Ok(RelationKind::RelatedTo),
        other => Err(PortError::Internal {
            message: format!("unknown relation kind {other}"),
        }),
    }
}

fn parse_relation_id(value: &str) -> Result<RelationId, PortError> {
    parse_u64(value, "relation id").map(RelationId::new)
}

fn parse_evidence_id(value: &str) -> Result<EvidenceId, PortError> {
    parse_u64(value, "evidence id").map(EvidenceId::new)
}

fn parse_u64(value: &str, label: &str) -> Result<u64, PortError> {
    value.parse::<u64>().map_err(|error| PortError::Internal {
        message: format!("stored {label} is invalid: {error}"),
    })
}

fn i64_to_u16(value: i64, label: &str) -> Result<u16, PortError> {
    u16::try_from(value).map_err(|_| PortError::Internal {
        message: format!("stored {label} {value} is outside u16 range"),
    })
}

pub(super) fn to_port_error(error: rusqlite::Error) -> PortError {
    PortError::Internal {
        message: format!("sqlite graph projection error: {error}"),
    }
}
