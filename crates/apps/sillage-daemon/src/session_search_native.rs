use sillage_search_tantivy::MemoryBudgetError;

use std::{
    ffi::{CString, OsStr},
    fs::File,
    path::{Component, Path},
};

#[derive(Debug)]
pub(super) enum NativeError {
    Io(std::io::Error),
    InvalidComponent,
    NotRegular,
    Admission,
    MemoryBudget(MemoryBudgetError),
    Cancelled,
    DeadlineExceeded,
    #[cfg(not(unix))]
    UnsupportedPlatform,
}

impl std::fmt::Display for NativeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "native file acquisition failed: {error}"),
            Self::InvalidComponent => formatter.write_str("selected path has an invalid component"),
            Self::NotRegular => formatter.write_str("selected source is not a regular file"),
            Self::Admission => formatter.write_str("source handle admission was refused"),
            Self::MemoryBudget(error) => write!(formatter, "{error}"),
            Self::Cancelled => formatter.write_str("source acquisition was cancelled"),
            Self::DeadlineExceeded => {
                formatter.write_str("source acquisition exceeded its selected deadline")
            }
            #[cfg(not(unix))]
            Self::UnsupportedPlatform => {
                formatter.write_str("selected-source acquisition is unsupported on this platform")
            }
        }
    }
}

impl std::error::Error for NativeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::MemoryBudget(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for NativeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AdmissionKind {
    Source,
    Scratch,
}

pub(super) trait AcquisitionAdmission {
    type Permit;

    fn reserve(&self, kind: AdmissionKind, bytes: u64) -> Result<Self::Permit, NativeError>;
    fn checkpoint(&self) -> Result<(), NativeError>;
}

pub(super) struct HandleIdentity {
    _device: u64,
    _object: u64,
}

impl std::fmt::Debug for HandleIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("HandleIdentity(<private>)")
    }
}

impl HandleIdentity {
    fn matches(&self, other: &Self) -> bool {
        self._device == other._device && self._object == other._object
    }
}

pub(super) struct AcquiredFile<P> {
    file: File,
    permit: P,
    identity: HandleIdentity,
    length: u64,
}

impl<P> AcquiredFile<P> {
    pub(super) fn file(&self) -> &File {
        &self.file
    }

    pub(super) fn length(&self) -> u64 {
        self.length
    }

    pub(super) fn into_parts(self) -> (File, P, HandleIdentity) {
        (self.file, self.permit, self.identity)
    }

    pub(super) fn matches_identity(&self, identity: &HandleIdentity) -> bool {
        self.identity.matches(identity)
    }
    #[cfg(all(test, unix))]
    pub(in crate::session_search) fn replace_file_for_test(mut self, file: File) -> Self {
        self.file = file;
        self
    }
}

struct AcquiredDirectory<P> {
    file: File,
    _permit: P,
}

#[cfg(unix)]
fn open_at(
    parent: std::os::fd::RawFd,
    name: &std::ffi::CStr,
    directory: bool,
) -> Result<File, NativeError> {
    use std::os::fd::FromRawFd;

    let mut flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    if directory {
        flags |= libc::O_DIRECTORY;
    }
    // SAFETY: name is NUL-terminated and parent is an owned directory handle or AT_FDCWD.
    let descriptor = unsafe { libc::openat(parent, name.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn c_component(name: &OsStr) -> Result<CString, NativeError> {
    use std::os::unix::ffi::OsStrExt;

    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(NativeError::InvalidComponent);
    }
    CString::new(bytes).map_err(|_| NativeError::InvalidComponent)
}

#[cfg(unix)]
fn open_root<A: AcquisitionAdmission>(
    admission: &A,
) -> Result<AcquiredDirectory<A::Permit>, NativeError> {
    use std::os::fd::AsRawFd;

    admission.checkpoint()?;
    let permit = admission.reserve(AdmissionKind::Scratch, 8192)?;
    let root_name = c"/";
    let file = open_at(libc::AT_FDCWD, root_name, true)?;
    let _ = file.as_raw_fd();
    Ok(AcquiredDirectory {
        file,
        _permit: permit,
    })
}

#[cfg(unix)]
fn open_directory<A: AcquisitionAdmission>(
    parent: &File,
    name: &OsStr,
    admission: &A,
) -> Result<AcquiredDirectory<A::Permit>, NativeError> {
    use std::os::fd::AsRawFd;

    admission.checkpoint()?;
    let _name_admission = admission.reserve(
        AdmissionKind::Scratch,
        u64::try_from(name.len())
            .map_err(|_| NativeError::Admission)?
            .saturating_add(1),
    )?;
    let name = c_component(name)?;
    let permit = admission.reserve(AdmissionKind::Scratch, 8192)?;
    let file = open_at(parent.as_raw_fd(), &name, true)?;
    if !file.metadata()?.is_dir() {
        return Err(NativeError::InvalidComponent);
    }
    Ok(AcquiredDirectory {
        file,
        _permit: permit,
    })
}

#[cfg(unix)]
fn open_file<A: AcquisitionAdmission>(
    parent: &File,
    name: &OsStr,
    admission: &A,
) -> Result<AcquiredFile<A::Permit>, NativeError> {
    use std::os::{fd::AsRawFd, unix::fs::MetadataExt};

    admission.checkpoint()?;
    let _name_admission = admission.reserve(
        AdmissionKind::Scratch,
        u64::try_from(name.len())
            .map_err(|_| NativeError::Admission)?
            .saturating_add(1),
    )?;
    let name = c_component(name)?;
    let permit = admission.reserve(AdmissionKind::Source, 8192)?;
    let file = open_at(parent.as_raw_fd(), &name, false)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(NativeError::NotRegular);
    }
    Ok(AcquiredFile {
        length: metadata.len(),
        identity: HandleIdentity {
            _device: metadata.dev(),
            _object: metadata.ino(),
        },
        file,
        permit,
    })
}

#[cfg(unix)]
pub(super) fn acquire_file<A: AcquisitionAdmission>(
    path: &Path,
    admission: &A,
) -> Result<AcquiredFile<A::Permit>, NativeError> {
    use std::os::unix::ffi::OsStrExt;

    #[cfg(all(test, unix))]
    pause_before_acquire_checkpoint(path);

    admission.checkpoint()?;
    let raw = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || raw.contains(&0)
        || raw
            .split(|byte| *byte == b'/')
            .any(|part| part == b"." || part == b"..")
    {
        return Err(NativeError::InvalidComponent);
    }
    let mut components = path.components().peekable();
    if components.next() != Some(Component::RootDir) {
        return Err(NativeError::InvalidComponent);
    }
    let mut parent = open_root(admission)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(NativeError::InvalidComponent);
        };
        if components.peek().is_none() {
            return open_file(&parent.file, name, admission);
        }
        parent = open_directory(&parent.file, name, admission)?;
    }
    Err(NativeError::NotRegular)
}

#[cfg(not(unix))]
pub(super) fn acquire_file<A: AcquisitionAdmission>(
    _path: &Path,
    _admission: &A,
) -> Result<AcquiredFile<A::Permit>, NativeError> {
    Err(NativeError::UnsupportedPlatform)
}

#[cfg(unix)]
pub(super) fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(buffer, offset)
}

#[cfg(windows)]
pub(super) fn read_at(file: &File, buffer: &mut [u8], offset: u64) -> std::io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_read(buffer, offset)
}

#[cfg(all(test, unix))]
pub(in crate::session_search) struct NativeAcquireTestGate {
    target: std::path::PathBuf,
    arrival: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    released: std::sync::Mutex<bool>,
    wake: std::sync::Condvar,
}

#[cfg(all(test, unix))]
static NATIVE_ACQUIRE_TEST_GATE: std::sync::Mutex<Option<std::sync::Arc<NativeAcquireTestGate>>> =
    std::sync::Mutex::new(None);

#[cfg(all(test, unix))]
impl NativeAcquireTestGate {
    pub(in crate::session_search) fn new(
        target: &Path,
    ) -> (std::sync::Arc<Self>, tokio::sync::oneshot::Receiver<()>) {
        let (arrival, reached) = tokio::sync::oneshot::channel();
        (
            std::sync::Arc::new(Self {
                target: target.to_path_buf(),
                arrival: std::sync::Mutex::new(Some(arrival)),
                released: std::sync::Mutex::new(false),
                wake: std::sync::Condvar::new(),
            }),
            reached,
        )
    }

    pub(in crate::session_search) fn install(gate: std::sync::Arc<Self>) -> anyhow::Result<()> {
        let mut current = lock_acquire_gate(&NATIVE_ACQUIRE_TEST_GATE);
        if current.is_some() {
            anyhow::bail!("a native-acquisition test gate is already installed");
        }
        *current = Some(gate);
        Ok(())
    }

    pub(in crate::session_search) fn release(&self) {
        let mut released = lock_acquire_gate(&self.released);
        *released = true;
        self.wake.notify_all();
    }

    pub(in crate::session_search) fn clear(expected: &std::sync::Arc<Self>) {
        let mut current = lock_acquire_gate(&NATIVE_ACQUIRE_TEST_GATE);
        if current
            .as_ref()
            .is_some_and(|installed| std::sync::Arc::ptr_eq(installed, expected))
        {
            current.take();
        }
    }

    fn pause(&self) {
        let arrival = lock_acquire_gate(&self.arrival).take();
        if let Some(arrival) = arrival
            && arrival.send(()).is_err()
        {
            self.release();
            return;
        }
        let mut released = lock_acquire_gate(&self.released);
        while !*released {
            released = match self.wake.wait(released) {
                Ok(released) => released,
                Err(poisoned) => poisoned.into_inner(),
            };
        }
    }
}

#[cfg(all(test, unix))]
fn pause_before_acquire_checkpoint(path: &Path) {
    let gate = {
        let mut current = lock_acquire_gate(&NATIVE_ACQUIRE_TEST_GATE);
        if current
            .as_ref()
            .is_some_and(|gate| gate.target.as_path() == path)
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
fn lock_acquire_gate<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}
