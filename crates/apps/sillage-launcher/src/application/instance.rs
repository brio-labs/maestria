use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tokio::time::Instant as MonotonicInstant;

use super::RuntimeMessage;
mod directory;
use directory::runtime_dir;

const CLIENT_IO_TIMEOUT: Duration = Duration::from_millis(500);
const CLIENT_POLL_INTERVAL: Duration = Duration::from_millis(5);
const LISTENER_POLL_INTERVAL: Duration = Duration::from_millis(20);
const ACK_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) struct PrimaryInstance {
    _socket: OwnedSocket,
    _lock: File,
    stopping: Arc<AtomicBool>,
    listener: Option<JoinHandle<()>>,
}

impl PrimaryInstance {
    pub(super) fn claim(
        messages: mpsc::SyncSender<RuntimeMessage>,
        activate_existing: bool,
    ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        Self::claim_in(&runtime_dir()?, messages, activate_existing)
    }

    fn claim_in(
        runtime_dir: &Path,
        messages: mpsc::SyncSender<RuntimeMessage>,
        activate_existing: bool,
    ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let uid = unsafe { libc::geteuid() };
        let lock_path = runtime_dir.join(format!("sillage-launcher-{uid}.lock"));
        let socket_path = runtime_dir.join(format!("sillage-launcher-{uid}.sock"));
        let lock_file = open_lock_file(&lock_path, uid)?;
        let result = unsafe { libc::flock(lock_file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                if !activate_existing {
                    return Ok(None);
                }
                if send_existing_in(runtime_dir, RuntimeMessage::Activate)? {
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

        remove_stale_socket(&socket_path, uid)?;
        let listener = UnixListener::bind(&socket_path)?;
        let socket = OwnedSocket::capture(socket_path, uid)?;
        socket.set_private_mode()?;
        listener.set_nonblocking(true)?;

        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stopping = Arc::clone(&stopping);
        let listener_thread = thread::Builder::new()
            .name("sillage-launcher-instance".to_string())
            .spawn(move || listen(listener, messages, thread_stopping))?;

        Ok(Some(Self {
            _lock: lock_file,
            _socket: socket,
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
        // `_socket` removes only the socket inode captured during successful bind.
    }
}

pub(super) fn send_existing(message: RuntimeMessage) -> Result<bool, Box<dyn std::error::Error>> {
    send_existing_in(&runtime_dir()?, message)
}

fn send_existing_in(
    runtime_dir: &Path,
    message: RuntimeMessage,
) -> Result<bool, Box<dyn std::error::Error>> {
    let uid = unsafe { libc::geteuid() };
    let socket_path = runtime_dir.join(format!("sillage-launcher-{uid}.sock"));
    let command = match message {
        RuntimeMessage::Activate => b'A',
        RuntimeMessage::Quit => b'Q',
    };

    for attempt in 0..40 {
        let expected = match socket_identity(&socket_path, uid) {
            Ok(identity) => identity,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };

        match UnixStream::connect(&socket_path) {
            Ok(mut stream) => {
                if !path_has_socket_identity(&socket_path, &expected)? {
                    return Ok(false);
                }
                stream.set_write_timeout(Some(CLIENT_IO_TIMEOUT))?;
                stream.set_read_timeout(Some(ACK_TIMEOUT))?;
                stream.write_all(&[command])?;
                let mut acknowledgement = [0u8; 1];
                let acknowledged =
                    stream.read_exact(&mut acknowledgement).is_ok() && acknowledgement[0] == 1;
                return Ok(acknowledged);
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

fn open_lock_file(path: &Path, uid: u32) -> io::Result<File> {
    let mut create = OpenOptions::new();
    create
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    let (file, created) = match create.open(path) {
        Ok(file) => (file, true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let path_metadata = fs::symlink_metadata(path)?;
            validate_lock_metadata(&path_metadata, uid)?;
            let mut open = OpenOptions::new();
            open.read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
            (open.open(path)?, false)
        }
        Err(error) => return Err(error),
    };

    let mut file_metadata = file.metadata()?;
    if created {
        if !file_metadata.file_type().is_file() || file_metadata.uid() != uid {
            return Err(permission_denied(
                "launcher lock is not a regular file owned by the current user",
            ));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file_metadata = file.metadata()?;
    }
    validate_lock_metadata(&file_metadata, uid)?;
    let path_metadata = fs::symlink_metadata(path)?;
    if !same_inode(&file_metadata, &path_metadata)
        || !path_metadata.file_type().is_file()
        || path_metadata.uid() != uid
    {
        return Err(permission_denied(
            "launcher lock path changed while it was opened",
        ));
    }
    validate_lock_metadata(&path_metadata, uid)?;
    Ok(file)
}

fn validate_lock_metadata(metadata: &fs::Metadata, uid: u32) -> io::Result<()> {
    if !metadata.file_type().is_file()
        || metadata.uid() != uid
        || metadata.permissions().mode() & 0o7777 != 0o600
    {
        return Err(permission_denied(
            "launcher lock is not a private regular file owned by the current user",
        ));
    }
    Ok(())
}

fn remove_stale_socket(path: &Path, uid: u32) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let identity = socket_identity_from_metadata(&metadata, uid, true)?;
    remove_socket_if_same(path, &identity)
}

struct SocketIdentity {
    dev: u64,
    ino: u64,
    uid: u32,
}

struct OwnedSocket {
    path: PathBuf,
    identity: SocketIdentity,
}

impl OwnedSocket {
    fn capture(path: PathBuf, uid: u32) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(&path)?;
        let identity = socket_identity_from_metadata(&metadata, uid, false)?;
        Ok(Self { path, identity })
    }

    fn set_private_mode(&self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !identity_matches(&metadata, &self.identity) {
            return Err(permission_denied("launcher socket path changed after bind"));
        }
        fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600))?;
        let metadata = fs::symlink_metadata(&self.path)?;
        if !identity_matches(&metadata, &self.identity)
            || metadata.permissions().mode() & 0o777 != 0o600
        {
            return Err(permission_denied(
                "launcher socket could not be made private",
            ));
        }
        Ok(())
    }
}

impl Drop for OwnedSocket {
    fn drop(&mut self) {
        let _ = remove_socket_if_same(&self.path, &self.identity);
    }
}

fn socket_identity(path: &Path, uid: u32) -> io::Result<SocketIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    socket_identity_from_metadata(&metadata, uid, true)
}

fn socket_identity_from_metadata(
    metadata: &fs::Metadata,
    uid: u32,
    require_private: bool,
) -> io::Result<SocketIdentity> {
    if !metadata.file_type().is_socket()
        || metadata.uid() != uid
        || (require_private && metadata.permissions().mode() & 0o777 != 0o600)
    {
        return Err(permission_denied(
            "launcher socket path is not an owned socket with private permissions",
        ));
    }
    Ok(SocketIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        uid: metadata.uid(),
    })
}

fn path_has_socket_identity(path: &Path, identity: &SocketIdentity) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(identity_matches(&metadata, identity)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn identity_matches(metadata: &fs::Metadata, identity: &SocketIdentity) -> bool {
    metadata.file_type().is_socket()
        && metadata.uid() == identity.uid
        && metadata.dev() == identity.dev
        && metadata.ino() == identity.ino
}

fn remove_socket_if_same(path: &Path, identity: &SocketIdentity) -> io::Result<()> {
    if !path_has_socket_identity(path, identity)? {
        return Ok(());
    }
    fs::remove_file(path)
}

fn same_inode(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

fn permission_denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

fn listen(
    listener: UnixListener,
    messages: mpsc::SyncSender<RuntimeMessage>,
    stopping: Arc<AtomicBool>,
) {
    while !stopping.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle_client(stream, &messages, &stopping),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(LISTENER_POLL_INTERVAL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                if !stopping.load(Ordering::Acquire) {
                    eprintln!("Launcher instance listener failed: {error}");
                }
                break;
            }
        }
    }
}

fn handle_client(
    mut stream: UnixStream,
    messages: &mpsc::SyncSender<RuntimeMessage>,
    stopping: &AtomicBool,
) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let Some(command) = read_command(&mut stream, stopping) else {
        return;
    };
    let message = match command {
        b'A' => RuntimeMessage::Activate,
        b'Q' => RuntimeMessage::Quit,
        _ => return,
    };
    if messages.try_send(message).is_ok() {
        let _ = write_acknowledgement(&mut stream, stopping);
    }
}

fn read_command(stream: &mut UnixStream, stopping: &AtomicBool) -> Option<u8> {
    let deadline = MonotonicInstant::now() + CLIENT_IO_TIMEOUT;
    let mut command = [0u8; 1];
    loop {
        if stopping.load(Ordering::Acquire) || MonotonicInstant::now() >= deadline {
            return None;
        }
        match stream.read(&mut command) {
            Ok(1) => return Some(command[0]),
            Ok(0) => return None,
            Ok(_) => return None,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(CLIENT_POLL_INTERVAL);
            }
            Err(_) => return None,
        }
    }
}

fn write_acknowledgement(stream: &mut UnixStream, stopping: &AtomicBool) -> io::Result<()> {
    let deadline = MonotonicInstant::now() + CLIENT_IO_TIMEOUT;
    loop {
        if stopping.load(Ordering::Acquire) || MonotonicInstant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "instance acknowledgement timed out",
            ));
        }
        match stream.write(&[1]) {
            Ok(1) => return Ok(()),
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "instance acknowledgement was not written",
                ));
            }
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(CLIENT_POLL_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests;
