use super::*;

impl InstanceManifest {
    pub fn encode(&self) -> String {
        let mut lines = vec![
            format!("schema_version={}", self.schema_version),
            format!("realm_id={}", self.realm_id.as_str()),
            format!("root={}", self.root.display()),
        ];
        lines.extend(
            self.read_roots
                .iter()
                .map(|root| format!("read_root={}", root.display())),
        );
        lines.extend(
            self.excluded_patterns
                .iter()
                .map(|pattern| format!("excluded_pattern={pattern}")),
        );
        if let Some(embeddings) = &self.embeddings {
            append_embedding(&mut lines, embeddings);
        }
        if let Some(ocr) = &self.ocr {
            append_ocr(&mut lines, ocr);
        }
        if let Some(visual) = &self.visual {
            append_visual(&mut lines, visual);
        }
        if let Some(late_interaction) = &self.late_interaction {
            append_late_interaction(&mut lines, late_interaction);
        }
        if let Some(sparse) = &self.sparse {
            append_sparse(&mut lines, sparse);
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

fn append_embedding(lines: &mut Vec<String>, config: &EmbeddingConfig) {
    lines.extend([
        format!("embedding_enabled={}", config.enabled),
        format!("embedding_endpoint={}", config.endpoint),
        format!("embedding_provider={}", config.provider),
        format!("embedding_revision={}", config.revision),
        format!("embedding_artifact_hash={}", config.artifact_hash),
        format!(
            "embedding_preprocessing_version={}",
            config.preprocessing_version
        ),
        format!("embedding_remote_provider={}", config.remote_provider),
        format!(
            "embedding_retention_policy={}",
            retention_policy_name(&config.retention_policy)
        ),
        format!("embedding_model={}", config.model),
        format!("embedding_dimensions={}", config.dimensions),
    ]);
}

fn append_ocr(lines: &mut Vec<String>, config: &OcrConfig) {
    lines.extend([
        format!("ocr_enabled={}", config.enabled),
        format!("ocr_endpoint={}", config.endpoint),
        format!("ocr_provider={}", config.provider),
        format!("ocr_revision={}", config.revision),
        format!("ocr_artifact_hash={}", config.artifact_hash),
        format!("ocr_preprocessing_version={}", config.preprocessing_version),
        format!("ocr_model={}", config.model),
    ]);
}

fn append_visual(lines: &mut Vec<String>, config: &VisualConfig) {
    lines.extend([
        format!("visual_enabled={}", config.enabled),
        format!("visual_endpoint={}", config.endpoint),
        format!("visual_provider={}", config.provider),
        format!("visual_revision={}", config.revision),
        format!("visual_artifact_hash={}", config.artifact_hash),
        format!(
            "visual_preprocessing_version={}",
            config.preprocessing_version
        ),
        format!("visual_remote_provider={}", config.remote_provider),
        format!(
            "visual_retention_policy={}",
            retention_policy_name(&config.retention_policy)
        ),
        format!("visual_model={}", config.model),
        format!("visual_dimensions={}", config.dimensions),
    ]);
}

fn append_late_interaction(lines: &mut Vec<String>, config: &LateInteractionConfig) {
    lines.extend([
        format!("late_interaction_endpoint={}", config.endpoint),
        format!(
            "late_interaction_profile_path={}",
            config.profile_path.display()
        ),
        format!(
            "late_interaction_mode={}",
            match config.mode {
                LateInteractionMode::Disabled => "disabled",
                LateInteractionMode::Shadow => "shadow",
                LateInteractionMode::Active => "active",
            }
        ),
    ]);
}

fn append_sparse(lines: &mut Vec<String>, config: &SparseProfileConfig) {
    lines.extend([
        format!("sparse_enabled={}", config.enabled),
        format!("sparse_endpoint={}", config.endpoint),
        format!("sparse_provider={}", config.provider),
        format!("sparse_revision={}", config.revision),
        format!("sparse_artifact_hash={}", config.artifact_hash),
        format!(
            "sparse_preprocessing_version={}",
            config.preprocessing_version
        ),
        format!("sparse_remote_provider={}", config.remote_provider),
        format!(
            "sparse_retention_policy={}",
            retention_policy_name(&config.retention_policy)
        ),
        format!("sparse_model={}", config.model),
        format!("sparse_vocabulary_size={}", config.vocabulary_size),
        format!("sparse_term_cap={}", config.term_cap),
    ]);
}
