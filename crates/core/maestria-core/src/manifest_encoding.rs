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
            lines.push(format!("embedding_enabled={}", embeddings.enabled));
            lines.push(format!("embedding_endpoint={}", embeddings.endpoint));
            lines.push(format!("embedding_provider={}", embeddings.provider));
            lines.push(format!("embedding_revision={}", embeddings.revision));
            lines.push(format!(
                "embedding_artifact_hash={}",
                embeddings.artifact_hash
            ));
            lines.push(format!(
                "embedding_preprocessing_version={}",
                embeddings.preprocessing_version
            ));
            lines.push(format!(
                "embedding_remote_provider={}",
                embeddings.remote_provider
            ));
            lines.push(format!(
                "embedding_retention_policy={}",
                retention_policy_name(&embeddings.retention_policy)
            ));
            lines.push(format!("embedding_model={}", embeddings.model));
            lines.push(format!("embedding_dimensions={}", embeddings.dimensions));
        }
        if let Some(ocr) = &self.ocr {
            lines.push(format!("ocr_enabled={}", ocr.enabled));
            lines.push(format!("ocr_endpoint={}", ocr.endpoint));
            lines.push(format!("ocr_provider={}", ocr.provider));
            lines.push(format!("ocr_revision={}", ocr.revision));
            lines.push(format!("ocr_artifact_hash={}", ocr.artifact_hash));
            lines.push(format!(
                "ocr_preprocessing_version={}",
                ocr.preprocessing_version
            ));
            lines.push(format!("ocr_model={}", ocr.model));
        }
        if let Some(visual) = &self.visual {
            lines.push(format!("visual_enabled={}", visual.enabled));
            lines.push(format!("visual_endpoint={}", visual.endpoint));
            lines.push(format!("visual_provider={}", visual.provider));
            lines.push(format!("visual_revision={}", visual.revision));
            lines.push(format!("visual_artifact_hash={}", visual.artifact_hash));
            lines.push(format!(
                "visual_preprocessing_version={}",
                visual.preprocessing_version
            ));
            lines.push(format!("visual_remote_provider={}", visual.remote_provider));
            lines.push(format!(
                "visual_retention_policy={}",
                retention_policy_name(&visual.retention_policy)
            ));
            lines.push(format!("visual_model={}", visual.model));
            lines.push(format!("visual_dimensions={}", visual.dimensions));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}
