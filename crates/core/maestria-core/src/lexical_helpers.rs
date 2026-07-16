use crate::ports::CorePorts;
use crate::provenance::{content_is_safe, evidence_id_for};
use maestria_domain::{Evidence, EvidenceKind};
use maestria_ports::{
    CardField, CardHit, ChunkField, FieldSelector, LexicalHitMetadata, LexicalQuery, MatchMode,
    SearchHit,
};

pub(super) struct CardCandidate {
    pub(super) hit: CardHit,
    pub(super) lexical_metadata: Option<LexicalHitMetadata>,
}

pub(super) struct ChunkCandidate {
    pub(super) hit: SearchHit,
    pub(super) lexical_metadata: Option<LexicalHitMetadata>,
}

pub(super) fn snapshot_id_from_evidence(evidence: &Evidence) -> Option<String> {
    match &evidence.kind {
        EvidenceKind::FileSpan {
            snapshot: Some(snapshot),
            ..
        }
        | EvidenceKind::PdfSpan { blob: snapshot, .. }
        | EvidenceKind::WebSnapshot { snapshot, .. }
        | EvidenceKind::CommandOutput { blob: snapshot, .. }
        | EvidenceKind::TestResult { log: snapshot, .. }
        | EvidenceKind::Diff {
            patch_blob: snapshot,
            ..
        } => Some(snapshot.value().to_string()),
        _ => None,
    }
}

pub(super) fn lexical_score(score: f32) -> u32 {
    if !score.is_finite() || score < 0.0 {
        return 0;
    }
    (score * 1_000.0) as u32
}

pub(super) fn vector_score(score: f32) -> u32 {
    if !score.is_finite() {
        return 0;
    }
    let normalized = ((score as f64 + 1.0) / 2.0).clamp(0.0, 1.0);
    (normalized * 1_000_000.0) as u32
}
pub(super) fn prepare_lexical_query(query: &str) -> (MatchMode, String) {
    let query = query.trim();
    if query.starts_with('"') && query.ends_with('"') && query.len() >= 2 {
        (
            MatchMode::Exact,
            query[1..query.len() - 1].trim().to_string(),
        )
    } else {
        (MatchMode::Contains, query.to_string())
    }
}

pub(super) fn card_lexical_query(
    q: &str,
    limit: usize,
    offset: usize,
    mode: MatchMode,
) -> LexicalQuery<CardField> {
    LexicalQuery {
        q: q.to_string(),
        limit,
        offset,
        mode,
        fields: vec![
            FieldSelector {
                field: CardField::Title,
                boost: 2.0,
            },
            FieldSelector {
                field: CardField::Body,
                boost: 1.0,
            },
            FieldSelector {
                field: CardField::Id,
                boost: 5.0,
            },
            FieldSelector {
                field: CardField::Path,
                boost: 3.0,
            },
            FieldSelector {
                field: CardField::Filename,
                boost: 4.0,
            },
            FieldSelector {
                field: CardField::Symbol,
                boost: 4.0,
            },
        ],
    }
}

pub(super) fn chunk_lexical_query(
    q: &str,
    limit: usize,
    offset: usize,
    mode: MatchMode,
) -> LexicalQuery<ChunkField> {
    LexicalQuery {
        q: q.to_string(),
        limit,
        offset,
        mode,
        fields: vec![
            FieldSelector {
                field: ChunkField::Text,
                boost: 1.0,
            },
            FieldSelector {
                field: ChunkField::Id,
                boost: 5.0,
            },
            FieldSelector {
                field: ChunkField::Path,
                boost: 3.0,
            },
            FieldSelector {
                field: ChunkField::Filename,
                boost: 4.0,
            },
            FieldSelector {
                field: ChunkField::Symbol,
                boost: 4.0,
            },
        ],
    }
}

pub(super) fn card_candidate_allowed(
    ports: &CorePorts<'_>,
    card_id: maestria_domain::CardId,
    artifact_id: maestria_domain::ArtifactId,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> bool {
    let Ok(Some(artifact)) = ports.artifacts.get(artifact_id) else {
        return false;
    };
    if policy.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed {
        return false;
    }
    let Ok(Some(card)) = ports.cards.get(card_id) else {
        return false;
    };
    policy.evaluate(&card.security) == maestria_governance::RetrievalDecision::Allowed
        && content_is_safe(&card.title)
        && content_is_safe(&card.body)
}

pub(super) fn chunk_candidate_allowed(
    ports: &CorePorts<'_>,
    chunk_id: maestria_domain::ChunkId,
    artifact_id: maestria_domain::ArtifactId,
    policy: &maestria_governance::RetrievalSecurityPolicy,
) -> bool {
    let Ok(Some(artifact)) = ports.artifacts.get(artifact_id) else {
        return false;
    };
    if policy.evaluate(&artifact.security) != maestria_governance::RetrievalDecision::Allowed {
        return false;
    }
    let Ok(Some(chunk)) = ports.chunks.get(chunk_id) else {
        return false;
    };
    let Ok(Some(evidence)) = ports
        .evidence
        .get(evidence_id_for(artifact_id, chunk.order))
    else {
        return false;
    };
    policy.evaluate(&evidence.security) == maestria_governance::RetrievalDecision::Allowed
        && content_is_safe(&chunk.text)
        && content_is_safe(&evidence.excerpt)
}
