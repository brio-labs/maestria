use std::sync::atomic::{AtomicU64, Ordering};

use super::*;

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);
mod durable {
    use super::*;

    /// Explicit durable documentary store for one selected source namespace.
    pub(in crate::session_search) struct RetainedDocumentaryStore {
        root: PathBuf,
        source_key: String,
    }

    impl RetainedDocumentaryStore {
        pub(in crate::session_search) fn new(root: PathBuf, source_path: &Path) -> Result<Self> {
            if !root.is_absolute()
                || root.as_os_str().is_empty()
                || root
                    .components()
                    .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
            {
                bail!("retention store root must be absolute and normalized");
            }
            let source_key = hash_key(source_path.as_os_str().as_encoded_bytes());
            Ok(Self { root, source_key })
        }

        pub(in crate::session_search) fn persist(
            &self,
            bytes: &[u8],
            content_hash: &ContentHash,
            memory: &Arc<RamMemoryBudget>,
        ) -> Result<PersistedGeneration> {
            let io_bytes = u64::try_from(self.root.as_os_str().len())?
                .checked_add(12_288)
                .ok_or_else(|| anyhow!("retained store admission overflow"))?;
            let _io_admission = memory.reserve(io_bytes)?;
            let root = open_store_root(&self.root, true)?
                .ok_or_else(|| anyhow!("explicit retention store root could not be opened"))?;
            let namespace =
                ensure_owned_directory(&root, ".sillage-session-store-v1", ".owner", STORE_MARKER)?;
            let sources = ensure_owned_directory(
                &namespace,
                "sources",
                ".owner",
                "sillage-source-namespaces-v1\n",
            )?;
            let source = ensure_owned_directory(
                &sources,
                &self.source_key,
                ".owner",
                &format!("{SOURCE_MARKER_PREFIX}{}\n", self.source_key),
            )?;
            let generations = ensure_owned_directory(
                &source,
                "generations",
                ".owner",
                "sillage-source-generations-v1\n",
            )?;
            let name = CString::new(generation_name(content_hash)?.into_bytes())?;
            match open_file_at(&generations, &name, false) {
                Ok(mut existing) => {
                    verify_private_file(&existing)?;
                    compare_file(&mut existing, bytes)?;
                    Ok(PersistedGeneration {
                        directory: generations,
                        name,
                        created: false,
                        committed: true,
                    })
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::AcqRel);
                    let staging_name = CString::new(
                        format!(".sillage-staging-{}-{sequence}", std::process::id()).into_bytes(),
                    )?;
                    let mut staging_guard =
                        StagingFile::create(generations.try_clone()?, staging_name)?;
                    staging_guard.file_mut()?.write_all(bytes)?;
                    staging_guard.file_mut()?.sync_all()?;
                    match link_at(&generations, &staging_guard.name, &name) {
                        Ok(()) => {
                            let generation = PersistedGeneration {
                                directory: generations.try_clone()?,
                                name,
                                created: true,
                                committed: false,
                            };
                            generations.sync_all()?;
                            staging_guard.remove()?;
                            Ok(generation)
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            let mut existing = open_file_at(&generations, &name, false)?;
                            verify_private_file(&existing)?;
                            compare_file(&mut existing, bytes)?;
                            staging_guard.remove()?;
                            Ok(PersistedGeneration {
                                directory: generations,
                                name,
                                created: false,
                                committed: true,
                            })
                        }
                        Err(error) => Err(error.into()),
                    }
                }
                Err(error) => Err(error.into()),
            }
        }
    }
    pub(in crate::session_search) struct PersistedGeneration {
        directory: File,
        name: CString,
        created: bool,
        committed: bool,
    }

    impl PersistedGeneration {
        pub(in crate::session_search) fn commit(&mut self) {
            self.committed = true;
        }
    }

    impl Drop for PersistedGeneration {
        fn drop(&mut self) {
            if self.created && !self.committed {
                let _ = unlink_at(&self.directory, &self.name, false);
            }
        }
    }

    struct StagingFile {
        directory: File,
        name: CString,
        file: Option<File>,
        removed: bool,
    }

    impl StagingFile {
        fn create(directory: File, name: CString) -> Result<Self> {
            let file = open_file_at(&directory, &name, true)
                .context("cannot create private retained-source staging file")?;
            set_private_file_mode(&file)?;
            verify_private_file(&file)?;
            Ok(Self {
                directory,
                name,
                file: Some(file),
                removed: false,
            })
        }

        fn file_mut(&mut self) -> Result<&mut File> {
            self.file
                .as_mut()
                .ok_or_else(|| anyhow!("retained-source staging file is closed"))
        }

        fn remove(&mut self) -> Result<()> {
            self.file.take();
            unlink_at(&self.directory, &self.name, false)?;
            self.removed = true;
            Ok(())
        }
    }

    impl Drop for StagingFile {
        fn drop(&mut self) {
            if !self.removed {
                self.file.take();
                let _ = unlink_at(&self.directory, &self.name, false);
            }
        }
    }

    fn compare_file(file: &mut File, expected: &[u8]) -> Result<()> {
        let metadata = file.metadata()?;
        verify_private_file(file)?;
        if !metadata.is_file() || metadata.len() != u64::try_from(expected.len())? {
            bail!("retained source generation conflicts with the selected content hash");
        }
        let mut offset: usize = 0;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let end = offset
                .checked_add(read)
                .ok_or_else(|| anyhow!("retained generation size overflow"))?;
            if expected.get(offset..end) != Some(&buffer[..read]) {
                bail!("retained source generation conflicts with the selected content hash");
            }
            offset = end;
        }
        if offset != expected.len() {
            bail!("retained source generation changed while being verified");
        }
        Ok(())
    }
}
fn generation_name(hash: &ContentHash) -> Result<String> {
    let value = hash
        .as_str()
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("source content hash has an unsupported representation"))?;
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("source content hash has an invalid representation");
    }
    Ok(format!("g-{value}.md"))
}
pub(in crate::session_search) use durable::{PersistedGeneration, RetainedDocumentaryStore};
