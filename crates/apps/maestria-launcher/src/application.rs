use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use slint::{ModelRc, VecModel};

use crate::model::SearchResult;
use crate::{ActionRow, LauncherWindow, ResultRow};

const UI_TICK: Duration = Duration::from_millis(50);
const SEARCH_DEBOUNCE_TICKS: u8 = 2;
const CATALOG_REFRESH_TICKS: u16 = 600;

mod callbacks;
mod dispatch;
mod extensions;
mod passages;
mod platform;
mod preferences;
mod runtime;
mod search;
mod window;

pub use runtime::run;
pub(super) fn empty_actions() -> ModelRc<ActionRow> {
    ModelRc::new(VecModel::default())
}

pub(super) fn empty_results() -> ModelRc<ResultRow> {
    ModelRc::new(VecModel::default())
}

pub(super) type UiWeak = slint::Weak<LauncherWindow>;

pub(super) enum RuntimeMessage {
    Activate,
    Quit,
}

#[derive(Clone)]
pub(super) struct FileSelection {
    result_id: String,
}
#[derive(Clone)]
struct AcceptedPassage {
    result_id: String,
    passage: passages::Passage,
}

#[derive(Clone)]
struct AcceptedPath {
    result_id: String,
    path: String,
}

pub(super) enum DisplayedResult {
    Application(usize),
    Group,
    Passage(usize),
    Path(usize),
}

pub(super) struct FrontendModel {
    query: String,
    pending_ticks: Option<u8>,
    accepted: Vec<SearchResult>,
    selected_file: Option<FileSelection>,
    catalog_ticks_until_refresh: u16,
    accepted_passages: Vec<AcceptedPassage>,
    accepted_paths: Vec<AcceptedPath>,
    displayed: Vec<DisplayedResult>,
    result_filter: String,
    content_view_passages: Vec<usize>,
}

pub(super) struct Frontend {
    generation: AtomicU64,
    model: Mutex<FrontendModel>,
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(super) fn has_argument(arguments: &[String], wanted: &str) -> bool {
    arguments.iter().any(|argument| argument == wanted)
}

#[cfg(target_os = "linux")]
mod instance {
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use super::RuntimeMessage;

    pub struct PrimaryInstance {
        _lock: File,
        socket_path: PathBuf,
        stopping: Arc<AtomicBool>,
        listener: Option<JoinHandle<()>>,
    }

    impl PrimaryInstance {
        pub fn claim(
            messages: mpsc::SyncSender<RuntimeMessage>,
        ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
            let runtime_dir = runtime_dir()?;
            let uid = unsafe { libc::geteuid() };
            let lock_path = runtime_dir.join(format!("maestria-launcher-{uid}.lock"));
            let socket_path = runtime_dir.join(format!("maestria-launcher-{uid}.sock"));
            let lock_file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .mode(0o600)
                .open(&lock_path)?;
            let result = unsafe {
                libc::flock(
                    std::os::fd::AsRawFd::as_raw_fd(&lock_file),
                    libc::LOCK_EX | libc::LOCK_NB,
                )
            };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    if send_existing(RuntimeMessage::Activate)? {
                        return Ok(None);
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "another launcher owns the instance lock but did not accept activation",
                    )
                    .into());
                }
                return Err(error.into());
            }

            if socket_path.exists() {
                fs::remove_file(&socket_path)?;
            }
            let listener = UnixListener::bind(&socket_path)?;
            fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            let stopping = Arc::new(AtomicBool::new(false));
            let thread_stopping = Arc::clone(&stopping);
            let thread_socket = socket_path.clone();
            let listener_thread = thread::Builder::new()
                .name("maestria-launcher-instance".to_string())
                .spawn(move || {
                    while !thread_stopping.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                let mut command = [0u8; 1];
                                if stream.read_exact(&mut command).is_ok() {
                                    let message = match command[0] {
                                        b'A' => Some(RuntimeMessage::Activate),
                                        b'Q' => Some(RuntimeMessage::Quit),
                                        _ => None,
                                    };
                                    if let Some(message) = message {
                                        let _ = messages.try_send(message);
                                        let _ = stream.write_all(&[1]);
                                    }
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(Duration::from_millis(20));
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) => {
                                if !thread_stopping.load(Ordering::Acquire) {
                                    eprintln!("Launcher instance listener failed: {error}");
                                }
                                break;
                            }
                        }
                    }
                    let _ = fs::remove_file(thread_socket);
                })?;

            Ok(Some(Self {
                _lock: lock_file,
                socket_path,
                stopping,
                listener: Some(listener_thread),
            }))
        }
    }

    impl Drop for PrimaryInstance {
        fn drop(&mut self) {
            self.stopping.store(true, Ordering::Release);
            if let Some(listener) = self.listener.take() {
                let _ = listener.join();
            }
            let _ = fs::remove_file(&self.socket_path);
        }
    }

    pub fn send_existing(message: RuntimeMessage) -> Result<bool, Box<dyn std::error::Error>> {
        let socket_path = runtime_dir()?.join(format!("maestria-launcher-{}.sock", unsafe {
            libc::geteuid()
        }));
        if !socket_path.exists() {
            return Ok(false);
        }
        let command = match message {
            RuntimeMessage::Activate => b'A',
            RuntimeMessage::Quit => b'Q',
        };
        for attempt in 0..40 {
            match UnixStream::connect(&socket_path) {
                Ok(mut stream) => {
                    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
                    stream.write_all(&[command])?;
                    let mut acknowledgement = [0u8; 1];
                    return Ok(
                        stream.read_exact(&mut acknowledgement).is_ok() && acknowledgement[0] == 1
                    );
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) && attempt < 39 =>
                {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(false)
    }

    fn runtime_dir() -> Result<PathBuf, io::Error> {
        let uid = unsafe { libc::geteuid() };
        if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
            let runtime = PathBuf::from(runtime);
            if runtime.is_absolute()
                && fs::symlink_metadata(&runtime).is_ok_and(|metadata| {
                    metadata.is_dir()
                        && metadata.uid() == uid
                        && metadata.permissions().mode() & 0o077 == 0
                })
            {
                return Ok(runtime);
            }
        }
        let fallback = std::env::temp_dir().join(format!("maestria-launcher-{uid}"));
        match fs::create_dir(&fallback) {
            Ok(()) => fs::set_permissions(&fallback, fs::Permissions::from_mode(0o700))?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(&fallback)?;
                if !metadata.is_dir()
                    || metadata.file_type().is_symlink()
                    || metadata.uid() != uid
                    || metadata.permissions().mode() & 0o077 != 0
                {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "launcher runtime directory is not private to the current user",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(fallback)
    }
}

#[cfg(not(target_os = "linux"))]
mod instance {
    use super::RuntimeMessage;
    use std::sync::mpsc;

    pub struct PrimaryInstance;

    impl PrimaryInstance {
        pub fn claim(
            _messages: mpsc::SyncSender<RuntimeMessage>,
        ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
            Ok(Some(Self))
        }
    }

    pub fn send_existing(_message: RuntimeMessage) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(false)
    }
}
