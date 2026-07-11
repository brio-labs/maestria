use crate::error::CoreError;
use crate::error::CoreResult;

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceLayout {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub database_path: PathBuf,
    pub blobs_dir: PathBuf,
    pub full_text_index_dir: PathBuf,
    pub vector_index_dir: PathBuf,
    pub graph_index_dir: PathBuf,
    pub workspace_dir: PathBuf,
    pub active_tasks_dir: PathBuf,
    pub system_dir: PathBuf,
    pub event_log_dir: PathBuf,
}

impl InstanceLayout {
    pub fn for_root(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let workspace_dir = root.join("workspace");
        let system_dir = root.join("system");
        Self {
            manifest_path: root.join("manifest.txt"),
            database_path: root.join("system").join("maestria.db"),
            blobs_dir: root.join("blobs").join("sha256"),
            full_text_index_dir: root.join("indexes").join("full-text"),
            vector_index_dir: root.join("indexes").join("vector"),
            graph_index_dir: root.join("indexes").join("graph"),
            active_tasks_dir: workspace_dir.join("active_tasks"),
            event_log_dir: system_dir.join("event_log"),
            workspace_dir,
            system_dir,
            root,
        }
    }
    pub fn required_directories(&self) -> Vec<PathBuf> {
        vec![
            self.root.clone(),
            self.blobs_dir.clone(),
            self.full_text_index_dir.clone(),
            self.vector_index_dir.clone(),
            self.graph_index_dir.clone(),
            self.workspace_dir.clone(),
            self.active_tasks_dir.clone(),
            self.system_dir.join("config"),
            self.system_dir.join("policies"),
            self.system_dir.join("logs"),
            self.system_dir.join("evidence_registry"),
            self.event_log_dir.clone(),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitInstanceInput {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitInstancePlan {
    pub layout: InstanceLayout,
    pub directories: Vec<PathBuf>,
    pub manifest_path: PathBuf,
    pub manifest_contents: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InstanceService;

impl InstanceService {
    pub fn init_instance(input: InitInstanceInput) -> CoreResult<InitInstancePlan> {
        Self::init_instance_with_roots(input.root.clone(), vec![input.root])
    }

    pub fn init_instance_with_roots(
        root: PathBuf,
        read_roots: Vec<PathBuf>,
    ) -> CoreResult<InitInstancePlan> {
        if root.as_os_str().is_empty() {
            return Err(CoreError::InvalidInput {
                message: "instance root must not be empty".to_string(),
            });
        }
        if read_roots.is_empty() || read_roots.iter().any(|path| path.as_os_str().is_empty()) {
            return Err(CoreError::InvalidInput {
                message: "instance must define at least one non-empty read root".to_string(),
            });
        }

        let layout = InstanceLayout::for_root(root);
        let mut manifest = crate::manifest::InstanceManifest::default_for_root(layout.root.clone());
        manifest.read_roots = read_roots;
        let manifest_contents = manifest.encode();
        Ok(InitInstancePlan {
            directories: layout.required_directories(),
            manifest_path: layout.manifest_path.clone(),
            manifest_contents,
            layout,
        })
    }

    pub fn parse_manifest(contents: &str) -> CoreResult<crate::manifest::InstanceManifest> {
        crate::manifest::InstanceManifest::decode(contents)
    }
}
