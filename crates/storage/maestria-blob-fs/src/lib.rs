#![forbid(unsafe_code)]

//! Content-addressed filesystem implementation of the Maestria blob port.

use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use maestria_domain::BlobId;
use maestria_ports::{BlobStore, PortError};
use sha2::{Digest, Sha256};

const DIGEST_HEX_LEN: usize = 64;

#[derive(Debug)]
pub struct FsBlobStore {
    root: PathBuf,
    temp_counter: AtomicU64,
}

impl FsBlobStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, PortError> {
        let root = root.as_ref();
        fs::create_dir_all(root).map_err(|error| io_error("create blob root", root, error))?;

        let root = root
            .canonicalize()
            .map_err(|error| io_error("canonicalize blob root", root, error))?;

        if !root.is_dir() {
            return Err(PortError::InvalidInput {
                message: format!("blob root is not a directory: {}", root.display()),
            });
        }

        for directory in [
            Self::blob_root(&root),
            Self::index_root(&root),
            Self::temp_root(&root),
        ] {
            fs::create_dir_all(&directory)
                .map_err(|error| io_error("create blob store directory", &directory, error))?;
        }

        Ok(Self {
            root,
            temp_counter: AtomicU64::new(0),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn put_with_digest(&self, bytes: Vec<u8>) -> Result<(BlobId, String), PortError> {
        let digest = sha256_digest(&bytes);
        let digest_hex = hex_digest(&digest);
        let id = id_from_digest(&digest);

        self.ensure_blob_file(&digest_hex, &bytes)?;
        self.ensure_id_mapping(id, &digest_hex)?;

        Ok((id, digest_hex))
    }

    pub fn digest_for_id(&self, id: BlobId) -> Result<String, PortError> {
        let path = self.index_path(id);
        let digest_hex = fs::read_to_string(&path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => PortError::NotFound,
            _ => io_error("read blob index", &path, error),
        })?;
        validate_digest_hex(digest_hex.trim()).map(str::to_owned)
    }

    pub fn object_path_for_digest(&self, digest_hex: &str) -> Result<PathBuf, PortError> {
        validate_digest_hex(digest_hex).map(|digest_hex| self.object_path(digest_hex))
    }

    fn ensure_id_mapping(&self, id: BlobId, digest_hex: &str) -> Result<(), PortError> {
        let path = self.index_path(id);
        match fs::read_to_string(&path) {
            Ok(existing) => {
                let existing = existing.trim();
                if existing == digest_hex {
                    Ok(())
                } else {
                    Err(PortError::Conflict {
                        message: format!(
                            "blob id {} maps to digest {existing}, not {digest_hex}",
                            id.value()
                        ),
                    })
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.link_new_index_file(&path, digest_hex.as_bytes())?;
                match fs::read_to_string(&path) {
                    Ok(existing) if existing.trim() == digest_hex => Ok(()),
                    Ok(existing) => Err(PortError::Conflict {
                        message: format!(
                            "blob id {} maps to digest {}, not {digest_hex}",
                            id.value(),
                            existing.trim()
                        ),
                    }),
                    Err(error) => Err(io_error("read blob index", &path, error)),
                }
            }
            Err(error) => Err(io_error("read blob index", &path, error)),
        }
    }

    fn ensure_blob_file(&self, digest_hex: &str, bytes: &[u8]) -> Result<(), PortError> {
        let path = self.object_path(digest_hex);
        if path.exists() {
            let existing =
                fs::read(&path).map_err(|error| io_error("verify existing blob", &path, error))?;
            if existing != bytes {
                return Err(PortError::Conflict {
                    message: format!("blob object content does not match digest {digest_hex}"),
                });
            }
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error("create blob object directory", parent, error))?;
        }

        self.rename_new_blob_file(&path, bytes)
    }

    fn rename_new_blob_file(&self, final_path: &Path, bytes: &[u8]) -> Result<(), PortError> {
        if final_path.exists() {
            return Ok(());
        }

        let temp_path = self.write_temp_file(bytes)?;
        let rename_result = if final_path.exists() {
            Ok(())
        } else {
            fs::rename(&temp_path, final_path)
                .map_err(|error| io_error("rename blob file", final_path, error))
        };

        match fs::remove_file(&temp_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if rename_result.is_ok() => {
                return Err(io_error("remove blob temp file", &temp_path, error));
            }
            Err(_) => {}
        }

        rename_result
    }

    fn link_new_index_file(&self, final_path: &Path, bytes: &[u8]) -> Result<(), PortError> {
        let temp_path = self.write_temp_file(bytes)?;
        let link_result = match fs::hard_link(&temp_path, final_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(io_error("link blob index", final_path, error)),
        };

        match fs::remove_file(&temp_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) if link_result.is_ok() => {
                return Err(io_error("remove blob temp file", &temp_path, error));
            }
            Err(_) => {}
        }

        link_result
    }

    fn write_temp_file(&self, bytes: &[u8]) -> Result<PathBuf, PortError> {
        for _ in 0..16 {
            let temp_path = self.temp_path();
            let mut temp_file = match File::create_new(&temp_path) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(io_error("create blob temp file", &temp_path, error)),
            };
            temp_file
                .write_all(bytes)
                .map_err(|error| io_error("write blob temp file", &temp_path, error))?;
            temp_file
                .sync_all()
                .map_err(|error| io_error("sync blob temp file", &temp_path, error))?;
            drop(temp_file);
            return Ok(temp_path);
        }

        Err(PortError::Internal {
            message: "could not allocate a unique blob temp file".to_string(),
        })
    }

    fn temp_path(&self) -> PathBuf {
        let count = self.temp_counter.fetch_add(1, Ordering::Relaxed);
        Self::temp_root(&self.root).join(format!("{}-{count}.tmp", std::process::id()))
    }

    fn object_path(&self, digest_hex: &str) -> PathBuf {
        Self::blob_root(&self.root)
            .join(&digest_hex[0..2])
            .join(&digest_hex[2..4])
            .join(digest_hex)
    }

    fn index_path(&self, id: BlobId) -> PathBuf {
        Self::index_root(&self.root).join(format!("{:016x}", id.value()))
    }

    fn blob_root(root: &Path) -> PathBuf {
        root.join("objects")
    }

    fn index_root(root: &Path) -> PathBuf {
        root.join("index")
    }

    fn temp_root(root: &Path) -> PathBuf {
        root.join("tmp")
    }
}

impl BlobStore for FsBlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
        self.put_with_digest(bytes).map(|(id, _)| id)
    }

    fn get(&self, id: BlobId) -> Result<Vec<u8>, PortError> {
        let digest_hex = self.digest_for_id(id)?;
        let path = self.object_path(&digest_hex);
        fs::read(&path).map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => PortError::NotFound,
            _ => io_error("read blob object", &path, error),
        })
    }
}

fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn id_from_digest(digest: &[u8; 32]) -> BlobId {
    let mut id_bytes = [0_u8; 8];
    id_bytes.copy_from_slice(&digest[0..8]);
    BlobId::new(u64::from_be_bytes(id_bytes))
}

fn hex_digest(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(DIGEST_HEX_LEN);
    for byte in digest {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    hex
}

fn validate_digest_hex(digest_hex: &str) -> Result<&str, PortError> {
    if digest_hex.len() == DIGEST_HEX_LEN
        && digest_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(digest_hex)
    } else {
        Err(PortError::InvalidInput {
            message: "blob digest must be 64 lowercase hexadecimal characters".to_string(),
        })
    }
}

fn io_error(action: &str, path: &Path, error: io::Error) -> PortError {
    PortError::Internal {
        message: format!("{action} at {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestria_ports::{BlobStore, contract_tests};
    use tempfile::tempdir;

    #[test]
    fn satisfies_shared_blob_store_contract() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let store = FsBlobStore::open(root.path())?;

        contract_tests::assert_blob_store_round_trip(&store)?;
        Ok(())
    }

    #[test]
    fn same_bytes_produce_same_id_and_digest() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let store = FsBlobStore::open(root.path())?;

        let first = store.put_with_digest(b"same bytes".to_vec())?;
        let second = store.put_with_digest(b"same bytes".to_vec())?;

        assert_eq!(first, second);
        assert_eq!(first.1, store.digest_for_id(first.0)?);
        Ok(())
    }

    #[test]
    fn different_bytes_round_trip() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let store = FsBlobStore::open(root.path())?;

        let first = store.put(b"first".to_vec())?;
        let second = store.put(b"second".to_vec())?;

        assert_ne!(first, second);
        assert_eq!(store.get(first)?, b"first");
        assert_eq!(store.get(second)?, b"second");
        Ok(())
    }

    #[test]
    fn missing_blob_returns_not_found() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let store = FsBlobStore::open(root.path())?;

        assert_eq!(store.get(BlobId::new(42)), Err(PortError::NotFound));
        Ok(())
    }

    #[test]
    fn stores_on_same_root_share_blobs() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let writer = FsBlobStore::open(root.path())?;
        let reader = FsBlobStore::open(root.path())?;

        let id = writer.put(b"shared".to_vec())?;

        assert_eq!(reader.get(id)?, b"shared");
        Ok(())
    }

    #[test]
    fn digest_derived_paths_stay_under_root() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempdir()?;
        let store = FsBlobStore::open(root.path())?;
        let (_, digest) = store.put_with_digest(b"caller cannot pick paths".to_vec())?;

        let object_path = store.object_path_for_digest(&digest)?;
        assert!(object_path.starts_with(store.root()));
        assert!(object_path.exists());
        assert!(
            object_path
                .strip_prefix(store.root())?
                .components()
                .all(|component| !matches!(component, std::path::Component::ParentDir))
        );

        let malicious = "../aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        assert!(matches!(
            store.object_path_for_digest(malicious),
            Err(PortError::InvalidInput { .. })
        ));
        Ok(())
    }
}
