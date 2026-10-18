use std::{
    collections::BTreeMap,
    path::{Component, Path},
    sync::{Arc, LazyLock, Mutex, Weak},
};

use anyhow::{Result, anyhow, bail};
use sillage_domain::SourceAvailability;
use sillage_search_tantivy::{RamMemoryBudget, RamMemoryReservation};

use super::control::SessionControl;

static SOURCE_CONTROLS: LazyLock<Mutex<BTreeMap<Arc<Path>, Weak<SessionControl>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn controls() -> &'static Mutex<BTreeMap<Arc<Path>, Weak<SessionControl>>> {
    &SOURCE_CONTROLS
}

pub(super) struct SourceLease {
    inner: Arc<SourceLeaseInner>,
}

struct SourceLeaseInner {
    path: Arc<Path>,
    control: Arc<SessionControl>,
    // Keep the charge until Drop removes the registry key and releases the lease path.
    _path_admission: RamMemoryReservation,
}

impl SourceLease {
    pub(super) fn acquire(
        path: &Path,
        availability: SourceAvailability,
        memory: &Arc<RamMemoryBudget>,
    ) -> Result<Self> {
        validate_source_path(path)?;
        let mut registry = controls()
            .lock()
            .map_err(|error| anyhow!("source ownership registry lock failed: {error}"))?;
        if let Some(active) = registry.get(path).and_then(Weak::upgrade)
            && !active.can_be_replaced()?
        {
            bail!("the selected source already has an active session");
        }
        let path_admission = memory.reserve(source_path_admission_bytes(path)?)?;
        let shared_path = Arc::<Path>::from(path);
        registry.remove(path);
        let control = Arc::new(SessionControl::new(availability));
        registry.insert(Arc::clone(&shared_path), Arc::downgrade(&control));
        Ok(Self {
            inner: Arc::new(SourceLeaseInner {
                path: shared_path,
                control,
                _path_admission: path_admission,
            }),
        })
    }

    pub(super) fn control(&self) -> Arc<SessionControl> {
        self.inner.control.clone()
    }

    pub(super) fn clone_lease(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for SourceLeaseInner {
    fn drop(&mut self) {
        let mut registry = match controls().lock() {
            Ok(registry) => registry,
            Err(poisoned) => poisoned.into_inner(),
        };
        if registry
            .get(self.path.as_ref())
            .and_then(Weak::upgrade)
            .is_some_and(|control| Arc::ptr_eq(&control, &self.control))
        {
            registry.remove(self.path.as_ref());
        }
    }
}

// Charge one shared path allocation, its Arc header, and both retained handles before allocation.
fn source_path_admission_bytes(path: &Path) -> Result<u64> {
    let handles = std::mem::size_of::<Arc<Path>>()
        .checked_mul(2)
        .ok_or_else(|| anyhow!("selected source lease path admission overflow"))?;
    let header = std::mem::size_of::<usize>()
        .checked_mul(2)
        .ok_or_else(|| anyhow!("selected source lease path admission overflow"))?;
    let overhead = handles
        .checked_add(header)
        .ok_or_else(|| anyhow!("selected source lease path admission overflow"))?;
    let path_bytes = path
        .as_os_str()
        .len()
        .checked_add(overhead)
        .ok_or_else(|| anyhow!("selected source lease path admission overflow"))?;
    u64::try_from(path_bytes).map_err(|error| {
        anyhow!("selected source lease path admission is not representable: {error}")
    })
}

pub(super) fn with_purge_exclusion<T>(path: &Path, purge: impl FnOnce() -> Result<T>) -> Result<T> {
    validate_source_path(path)?;
    let registry = controls()
        .lock()
        .map_err(|error| anyhow!("source ownership registry lock failed: {error}"))?;
    if let Some(active) = registry.get(path).and_then(Weak::upgrade)
        && !active.can_be_purged()?
    {
        bail!("the selected source must be fully retired before documentary purge");
    }
    purge()
}

pub(super) fn validate_source_path(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || path.to_str().is_none()
    {
        bail!("selected source path must be absolute, normalized UTF-8");
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_search_source_admission_tests.rs"]
mod tests;
