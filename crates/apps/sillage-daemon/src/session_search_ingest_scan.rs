use std::{
    fs::File,
    path::{Component, Path},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow, bail};
use sillage_domain::{ContentHash, SourceChoice, content_hash};
use sillage_search_tantivy::RamMemoryBudget;
use tokio_util::sync::CancellationToken;

use super::{SessionBuilder, SessionDeadline, ensure_not_cancelled};
use crate::session_search::{
    native,
    retained::{Admission, RetainedBinding},
};

pub(super) fn validate_selected_path(
    scope: &sillage_domain::SourceScope,
) -> Result<(&Path, &Path)> {
    let path = scope.path();
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || path.to_str().is_none()
    {
        bail!("selected source has an unsupported documentary path");
    }
    if scope.selection() != sillage_domain::SourceSelection::File {
        bail!("on-demand session preparation requires one explicitly selected file");
    }
    let extension = path.extension().and_then(|value| value.to_str());
    if !extension.is_some_and(|value| {
        value.eq_ignore_ascii_case("md") || value.eq_ignore_ascii_case("markdown")
    }) {
        bail!("on-demand session preparation supports Markdown files only");
    }
    let root = path
        .parent()
        .ok_or_else(|| anyhow!("selected source has no parent directory"))?;
    Ok((path, root))
}

pub(super) fn scan_selected_source(
    choice: &SourceChoice,
    selected_path: &Path,
    cancellation: &CancellationToken,
    builder: &mut SessionBuilder<'_>,
) -> Result<()> {
    ensure_not_cancelled(cancellation)?;
    if choice.scope().path() != selected_path
        || choice.scope().selection() != sillage_domain::SourceSelection::File
    {
        bail!("selected source escaped its explicit file scope");
    }
    if sillage_index_selection::is_privacy_excluded_path(selected_path) {
        bail!("selected source is excluded by the local privacy policy");
    }
    builder.coverage.visited_entries = 1;
    let memory = builder.memory();
    let admission = Admission {
        memory: &memory,
        cancellation,
        deadline: builder.deadline(),
    };
    let file = native::acquire_file(selected_path, &admission).map_err(|error| {
        anyhow::Error::new(error).context("cannot acquire the selected Markdown source")
    })?;
    builder.visit_file(selected_path, file, cancellation)
}

pub(in crate::session_search) struct RetainedSourceValidation<'a> {
    pub(in crate::session_search) binding: &'a RetainedBinding,
    pub(in crate::session_search) max_bytes: u64,
    pub(in crate::session_search) expected_hash: &'a ContentHash,
    pub(in crate::session_search) cancellation: &'a CancellationToken,
    pub(in crate::session_search) deadline: SessionDeadline,
    pub(in crate::session_search) memory: &'a Arc<RamMemoryBudget>,
    pub(in crate::session_search) max_work: u64,
    pub(in crate::session_search) work: &'a Mutex<u64>,
}

pub(in crate::session_search) fn verify_retained_source(
    validation: &RetainedSourceValidation<'_>,
) -> Result<()> {
    if &validation.binding.hash != validation.expected_hash {
        return Err(super::super::StaleEvidence(
            "retained source binding does not match the indexed snapshot",
        )
        .into());
    }
    let admission = Admission {
        memory: validation.memory,
        cancellation: validation.cancellation,
        deadline: validation.deadline,
    };
    let current = native::acquire_file(&validation.binding.path, &admission)
        .map_err(anyhow::Error::new)
        .context("current selected source cannot be reopened")?;
    if !validation.binding.source.matches_identity(&current) {
        return Err(super::super::StaleEvidence(
            "selected source identity changed after preparation",
        )
        .into());
    }
    let metadata = current.file().metadata()?;
    if !metadata.is_file() || metadata.len() > validation.max_bytes {
        return Err(super::super::StaleEvidence(
            "selected source is no longer an allowed bounded file",
        )
        .into());
    }
    let _read = validation.memory.reserve(
        metadata
            .len()
            .checked_add(16_385)
            .ok_or_else(|| anyhow!("session source verification admission overflow"))?,
    )?;
    let bytes = read_source_bounded(
        current.file(),
        metadata.len(),
        validation.cancellation,
        validation.deadline,
        |count| {
            let mut charged = validation
                .work
                .lock()
                .map_err(|_| anyhow!("session verification work lock failed"))?;
            let next = charged
                .checked_add(count)
                .ok_or_else(super::verification::work_budget_exceeded)?;
            *charged = next;
            if next > validation.max_work {
                return Err(super::verification::work_budget_exceeded());
            }
            Ok(())
        },
    )?;
    if bytes.len() as u64 != metadata.len() {
        return Err(super::super::StaleEvidence(
            "selected source changed while its contents were verified",
        )
        .into());
    }
    if ContentHash::new(content_hash(&bytes))? != validation.binding.hash {
        return Err(super::super::StaleEvidence(
            "selected source contents no longer match the indexed snapshot",
        )
        .into());
    }
    Ok(())
}

pub(in crate::session_search) fn scope_contains(
    scope: &sillage_domain::SourceScope,
    selected: &Path,
    path: &Path,
) -> bool {
    scope.selection() == sillage_domain::SourceSelection::File && path == selected
}

pub(super) fn read_source_bounded(
    file: &File,
    max_bytes: u64,
    cancellation: &CancellationToken,
    deadline: SessionDeadline,
    mut charge_work: impl FnMut(u64) -> Result<()>,
) -> Result<Vec<u8>> {
    let size = usize::try_from(max_bytes)
        .map_err(|error| anyhow!("selected source is not addressable: {error}"))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(size)?;
    bytes.resize(size, 0);
    let mut offset = 0_u64;
    while offset < max_bytes {
        ensure_read_active(cancellation, deadline)?;
        let remaining = max_bytes - offset;
        let length = usize::try_from(remaining.min(16_384))?;
        let start = usize::try_from(offset)?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| anyhow!("selected source read range overflow"))?;
        #[cfg(all(test, unix))]
        pause_before_native_read(file);

        let read = native::read_at(file, &mut bytes[start..end], offset)?;
        if read == 0 {
            bytes.truncate(start);
            break;
        }
        charge_work(u64::try_from(read)?)?;
        offset = offset
            .checked_add(u64::try_from(read)?)
            .ok_or_else(|| anyhow!("selected source offset overflow"))?;
    }
    ensure_read_active(cancellation, deadline)?;
    Ok(bytes)
}

fn ensure_read_active(cancellation: &CancellationToken, deadline: SessionDeadline) -> Result<()> {
    if cancellation.is_cancelled() {
        return Err(crate::SessionOperationSettlement::RoutineCancellation.into());
    }
    if deadline.is_elapsed() {
        return Err(crate::SessionOperationSettlement::DeadlineExceeded.into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
pub(in crate::session_search) struct NativeReadTestGate {
    device: u64,
    object: u64,
    arrival: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: Mutex<bool>,
    wake: std::sync::Condvar,
}

#[cfg(all(test, unix))]
static NATIVE_READ_TEST_GATE: Mutex<Option<Arc<NativeReadTestGate>>> = Mutex::new(None);

#[cfg(all(test, unix))]
impl NativeReadTestGate {
    pub(in crate::session_search) fn new(
        file: &File,
    ) -> Result<(Arc<Self>, tokio::sync::oneshot::Receiver<()>)> {
        use std::os::unix::fs::MetadataExt;

        let metadata = file.metadata()?;
        let (arrival, reached) = tokio::sync::oneshot::channel();
        Ok((
            Arc::new(Self {
                device: metadata.dev(),
                object: metadata.ino(),
                arrival: Mutex::new(Some(arrival)),
                released: Mutex::new(false),
                wake: std::sync::Condvar::new(),
            }),
            reached,
        ))
    }

    pub(in crate::session_search) fn install(gate: Arc<Self>) -> Result<()> {
        let mut current = lock_read_gate(&NATIVE_READ_TEST_GATE);
        if current.is_some() {
            bail!("a native-read test gate is already installed");
        }
        *current = Some(gate);
        Ok(())
    }

    pub(in crate::session_search) fn release(&self) {
        let mut released = lock_read_gate(&self.released);
        *released = true;
        self.wake.notify_all();
    }

    pub(in crate::session_search) fn clear(expected: &Arc<Self>) {
        let mut current = lock_read_gate(&NATIVE_READ_TEST_GATE);
        if current
            .as_ref()
            .is_some_and(|installed| Arc::ptr_eq(installed, expected))
        {
            current.take();
        }
    }

    fn pause(&self) {
        let arrival = lock_read_gate(&self.arrival).take();
        if let Some(arrival) = arrival
            && arrival.send(()).is_err()
        {
            self.release();
            return;
        }
        let mut released = lock_read_gate(&self.released);
        while !*released {
            released = match self.wake.wait(released) {
                Ok(released) => released,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }
}

#[cfg(all(test, unix))]
fn pause_before_native_read(file: &File) {
    use std::os::unix::fs::MetadataExt;

    let identity = file
        .metadata()
        .ok()
        .map(|metadata| (metadata.dev(), metadata.ino()));
    let gate = {
        let mut current = lock_read_gate(&NATIVE_READ_TEST_GATE);
        if current
            .as_ref()
            .is_some_and(|gate| identity == Some((gate.device, gate.object)))
        {
            current.take()
        } else {
            None
        }
    };
    if let Some(gate) = gate {
        gate.pause();
    }
}

#[cfg(all(test, unix))]
fn lock_read_gate<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
