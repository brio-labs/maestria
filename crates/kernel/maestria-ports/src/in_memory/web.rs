use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use super::store::lock_map;
use crate::{PortError, WebFetcher, WebSnapshotData};

#[derive(Clone, Default)]
pub struct InMemoryWebFetcher {
    pages: Arc<Mutex<BTreeMap<String, String>>>,
}

impl InMemoryWebFetcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(&self, url: &str, html: &str) -> Result<(), PortError> {
        let mut guard = lock_map(&self.pages, "web fetcher lock poisoned")?;
        guard.insert(url.to_string(), html.to_string());
        Ok(())
    }
}

impl WebFetcher for InMemoryWebFetcher {
    fn fetch(&self, url: &str, max_bytes: usize) -> Result<WebSnapshotData, PortError> {
        if url.trim().is_empty() {
            return Err(PortError::InvalidInputContext {
                context: "web fetch URL is empty",
                source: "URL must contain a non-whitespace value".to_string(),
            });
        }
        if max_bytes == 0 {
            return Err(PortError::InvalidInputContext {
                context: "web fetch byte limit is zero",
                source: "max_bytes must be greater than zero".to_string(),
            });
        }
        let guard = lock_map(&self.pages, "web fetcher lock poisoned")?;
        if let Some(html) = guard.get(url) {
            if html.len() > max_bytes {
                return Err(PortError::InvalidInputContext {
                    context: "web response exceeds byte limit",
                    source: "response length exceeds max_bytes".to_string(),
                });
            }
            Ok(WebSnapshotData {
                url: url.to_string(),
                content_hash: maestria_domain::content_hash(html.as_bytes()),
                html: html.clone(),
                metadata: maestria_domain::WebEvidenceMetadata::default(),
            })
        } else {
            Err(PortError::NotFound)
        }
    }
}
