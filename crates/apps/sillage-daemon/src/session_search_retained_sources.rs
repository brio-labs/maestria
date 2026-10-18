use std::{
    fmt,
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Result, anyhow, bail};
use sillage_domain::{ArtifactId, ArtifactVersionId, ContentHash};
use sillage_search_tantivy::{RamMemoryBudget, RamMemoryReservation};
use tokio_util::sync::CancellationToken;

use super::super::native::{
    AcquiredFile, AcquisitionAdmission, AdmissionKind, HandleIdentity, NativeError,
};

const MAX_RETAINED_SOURCE_HANDLES: usize = 64;
static SOURCE_HANDLES: AtomicUsize = AtomicUsize::new(0);
mod source_state {
    use super::*;

    struct SlotPermit;

    impl SlotPermit {
        fn reserve() -> Result<Self, NativeError> {
            SOURCE_HANDLES
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    (used < MAX_RETAINED_SOURCE_HANDLES).then_some(used + 1)
                })
                .map_err(|_| NativeError::Admission)?;
            Ok(Self)
        }
    }

    impl Drop for SlotPermit {
        fn drop(&mut self) {
            SOURCE_HANDLES.fetch_sub(1, Ordering::AcqRel);
        }
    }

    pub(in crate::session_search) struct OwnershipPermit {
        _slot: Option<SlotPermit>,
        _memory: RamMemoryReservation,
    }

    pub(in crate::session_search) struct Admission<'a> {
        pub(in crate::session_search) memory: &'a Arc<RamMemoryBudget>,
        pub(in crate::session_search) cancellation: &'a CancellationToken,
        pub(in crate::session_search) deadline: crate::session_search::deadline::SessionDeadline,
    }

    impl AcquisitionAdmission for Admission<'_> {
        type Permit = OwnershipPermit;

        fn reserve(&self, kind: AdmissionKind, bytes: u64) -> Result<Self::Permit, NativeError> {
            self.checkpoint()?;
            let slot = if kind == AdmissionKind::Source {
                Some(SlotPermit::reserve()?)
            } else {
                None
            };
            let overhead = std::mem::size_of::<RetainedSourceObject>() as u64 + 128;
            let memory = self
                .memory
                .reserve(bytes.checked_add(overhead).ok_or(NativeError::Admission)?)
                .map_err(NativeError::MemoryBudget)?;
            Ok(OwnershipPermit {
                _slot: slot,
                _memory: memory,
            })
        }

        fn checkpoint(&self) -> Result<(), NativeError> {
            if self.cancellation.is_cancelled() {
                return Err(NativeError::Cancelled);
            }
            if self.deadline.is_elapsed() {
                return Err(NativeError::DeadlineExceeded);
            }
            Ok(())
        }
    }

    /// Opaque ownership of the selected original file handle; debug output never reveals its path.
    pub(in crate::session_search) struct RetainedSourceObject {
        _file: File,
        _permit: OwnershipPermit,
        _identity: HandleIdentity,
    }

    impl fmt::Debug for RetainedSourceObject {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("RetainedSourceObject { .. }")
        }
    }

    impl RetainedSourceObject {
        pub(in crate::session_search) fn from_acquired(
            acquired: AcquiredFile<OwnershipPermit>,
        ) -> Arc<Self> {
            let (file, permit, identity) = acquired.into_parts();
            Arc::new(Self {
                _file: file,
                _permit: permit,
                _identity: identity,
            })
        }

        pub(in crate::session_search) fn matches_identity<P>(
            &self,
            current: &AcquiredFile<P>,
        ) -> bool {
            current.matches_identity(&self._identity)
        }
    }

    pub(in crate::session_search) struct RetainedBinding {
        artifact_id: ArtifactId,
        version: ArtifactVersionId,
        pub(in crate::session_search) path: PathBuf,
        pub(in crate::session_search) hash: ContentHash,
        pub(in crate::session_search) source: Arc<RetainedSourceObject>,
        _memory: RamMemoryReservation,
    }

    pub(in crate::session_search) struct RetainedSources {
        bindings: Vec<RetainedBinding>,
        map_memory: Option<RamMemoryReservation>,
    }

    impl RetainedSources {
        pub(in crate::session_search) fn new() -> Self {
            Self {
                bindings: Vec::new(),
                map_memory: None,
            }
        }

        pub(in crate::session_search) fn binding(
            &self,
            artifact: ArtifactId,
            version: ArtifactVersionId,
            path: &Path,
        ) -> Result<&RetainedBinding> {
            self.bindings
                .iter()
                .find(|binding| {
                    binding.artifact_id == artifact
                        && binding.version == version
                        && binding.path == path
                })
                .ok_or_else(|| anyhow!("session evidence has no retained original source binding"))
        }

        pub(in crate::session_search) fn admit_binding(
            &mut self,
            artifact: ArtifactId,
            version: ArtifactVersionId,
            path: &Path,
            memory: &Arc<RamMemoryBudget>,
        ) -> Result<RamMemoryReservation> {
            if self.bindings.iter().any(|binding| {
                binding.artifact_id == artifact
                    && binding.version == version
                    && binding.path == path
            }) {
                bail!("conflicting retained original source binding");
            }
            let path_bytes = u64::try_from(path.as_os_str().len())?;
            let binding_memory = memory.reserve(path_bytes.saturating_add(512))?;
            if self.bindings.len() == self.bindings.capacity() {
                let capacity = self
                    .bindings
                    .capacity()
                    .max(4)
                    .checked_mul(2)
                    .ok_or_else(|| anyhow!("retained source map capacity overflow"))?;
                let estimate = u64::try_from(capacity)?
                    .checked_mul(std::mem::size_of::<RetainedBinding>() as u64 + 64)
                    .and_then(|bytes| bytes.checked_add(128))
                    .ok_or_else(|| anyhow!("retained source map admission overflow"))?;
                let admitted = memory.reserve(estimate)?;
                self.bindings
                    .try_reserve_exact(capacity - self.bindings.len())?;
                self.map_memory = Some(admitted);
            }
            Ok(binding_memory)
        }

        pub(in crate::session_search) fn publish(
            &mut self,
            artifact: ArtifactId,
            version: ArtifactVersionId,
            path: PathBuf,
            hash: ContentHash,
            source: Arc<RetainedSourceObject>,
            memory: RamMemoryReservation,
        ) -> Result<()> {
            if self.bindings.iter().any(|binding| {
                binding.artifact_id == artifact
                    && binding.version == version
                    && binding.path == path
            }) {
                bail!("conflicting retained original source binding");
            }
            self.bindings.push(RetainedBinding {
                artifact_id: artifact,
                version,
                path,
                hash,
                source,
                _memory: memory,
            });
            Ok(())
        }
    }
}
pub(in crate::session_search) use source_state::{
    Admission, OwnershipPermit, RetainedBinding, RetainedSourceObject, RetainedSources,
};
