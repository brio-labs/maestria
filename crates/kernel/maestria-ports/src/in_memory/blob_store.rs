use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use crate::{BlobId, PortError};

#[derive(Clone, Default)]
pub struct InMemoryBlobStore {
    blobs: Arc<Mutex<BTreeMap<BlobId, Vec<u8>>>>,
    ids_by_content: Arc<Mutex<BTreeMap<Vec<u8>, BlobId>>>,
    next_id: Arc<Mutex<u64>>,
}

impl InMemoryBlobStore {
    pub fn new() -> Self {
        Self {
            blobs: Default::default(),
            ids_by_content: Default::default(),
            next_id: Arc::new(Mutex::new(1)),
        }
    }
}

impl crate::BlobStore for InMemoryBlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<BlobId, PortError> {
        let mut index_guard = self
            .ids_by_content
            .lock()
            .map_err(|_| PortError::Internal {
                message: "blob store lock poisoned".to_string(),
            })?;
        if let Some(id) = index_guard.get(&bytes) {
            return Ok(*id);
        }

        let mut id_guard = self.next_id.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;
        let mut blob_guard = self.blobs.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;

        let id = BlobId::new(*id_guard);
        *id_guard = id.value().saturating_add(1);
        blob_guard.insert(id, bytes.clone());
        index_guard.insert(bytes, id);
        Ok(id)
    }

    fn get(&self, id: BlobId) -> Result<Vec<u8>, PortError> {
        let guard = self.blobs.lock().map_err(|_| PortError::Internal {
            message: "blob store lock poisoned".to_string(),
        })?;
        guard.get(&id).cloned().ok_or(PortError::NotFound)
    }
}
