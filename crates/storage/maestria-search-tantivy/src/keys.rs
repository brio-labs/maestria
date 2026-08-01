use maestria_domain::{ArtifactId, CardId, ChunkId};

pub(crate) fn chunk_key(artifact_id: ArtifactId, chunk_id: ChunkId) -> String {
    format!("{}:{}", artifact_id.value(), chunk_id.value())
}

pub(crate) fn card_key(artifact_id: ArtifactId, card_id: CardId) -> String {
    format!("card:{}:{}", artifact_id.value(), card_id.value())
}
