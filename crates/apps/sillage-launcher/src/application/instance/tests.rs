use super::*;
use std::sync::atomic::AtomicU64;

static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> io::Result<Self> {
        let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "sillage-instance-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn socket_path(directory: &TestDirectory) -> PathBuf {
    directory
        .path()
        .join(format!("sillage-launcher-{}.sock", unsafe {
            libc::geteuid()
        }))
}

#[test]
fn idle_client_does_not_block_bounded_instance_shutdown() -> io::Result<()> {
    let directory = TestDirectory::new()?;
    let (sender, _receiver) = mpsc::sync_channel(1);
    let instance = PrimaryInstance::claim_in(directory.path(), sender, true)
        .map_err(|error| io::Error::other(error.to_string()))?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "first process should own the test instance",
            )
        })?;
    let idle_client = UnixStream::connect(socket_path(&directory))?;
    // Allow the listener's short poll interval to accept and wait on this idle stream.
    thread::sleep(Duration::from_millis(100));
    let (done_sender, done_receiver) = mpsc::channel();
    thread::spawn(move || {
        drop(instance);
        let _ = done_sender.send(());
    });
    done_receiver
        .recv_timeout(Duration::from_millis(400))
        .map_err(|error| io::Error::other(format!("instance shutdown blocked: {error}")))?;
    drop(idle_client);
    Ok(())
}

#[test]
fn full_command_queue_is_not_acknowledged() -> io::Result<()> {
    let directory = TestDirectory::new()?;
    let (sender, receiver) = mpsc::sync_channel(0);
    let instance = PrimaryInstance::claim_in(directory.path(), sender, true)
        .map_err(|error| io::Error::other(error.to_string()))?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "first process should own the test instance",
            )
        })?;

    assert!(
        !send_existing_in(directory.path(), RuntimeMessage::Activate)
            .map_err(|error| io::Error::other(error.to_string()))?
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(instance);
    Ok(())
}

#[test]
fn socket_replacement_survives_instance_cleanup() -> io::Result<()> {
    let directory = TestDirectory::new()?;
    let (sender, _receiver) = mpsc::sync_channel(1);
    let instance = PrimaryInstance::claim_in(directory.path(), sender, true)
        .map_err(|error| io::Error::other(error.to_string()))?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "first process should own the test instance",
            )
        })?;
    let path = socket_path(&directory);
    fs::remove_file(&path)?;
    fs::write(&path, b"replacement remains")?;

    drop(instance);

    assert_eq!(fs::read(&path)?, b"replacement remains");
    Ok(())
}

#[test]
fn symlink_lock_and_foreign_socket_are_preserved() -> io::Result<()> {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new()?;
    let uid = unsafe { libc::geteuid() };
    let lock_target = directory.path().join("lock-target");
    fs::write(&lock_target, b"lock target")?;
    symlink(
        &lock_target,
        directory
            .path()
            .join(format!("sillage-launcher-{uid}.lock")),
    )?;
    let (sender, _receiver) = mpsc::sync_channel(1);
    assert!(PrimaryInstance::claim_in(directory.path(), sender, true).is_err());
    assert_eq!(fs::read(&lock_target)?, b"lock target");

    fs::remove_file(
        directory
            .path()
            .join(format!("sillage-launcher-{uid}.lock")),
    )?;
    let foreign_socket = socket_path(&directory);
    let socket_target = directory.path().join("socket-target");
    fs::write(&socket_target, b"socket target")?;
    symlink(&socket_target, &foreign_socket)?;
    let (sender, _receiver) = mpsc::sync_channel(1);
    assert!(PrimaryInstance::claim_in(directory.path(), sender, true).is_err());
    assert!(
        fs::symlink_metadata(&foreign_socket)?
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&socket_target)?, b"socket target");

    fs::remove_file(&foreign_socket)?;
    fs::write(&foreign_socket, b"foreign socket path")?;
    let (sender, _receiver) = mpsc::sync_channel(1);
    assert!(PrimaryInstance::claim_in(directory.path(), sender, true).is_err());
    assert_eq!(fs::read(&foreign_socket)?, b"foreign socket path");
    Ok(())
}

#[test]
fn background_second_instance_does_not_activate_existing_instance() -> io::Result<()> {
    let directory = TestDirectory::new()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let instance = PrimaryInstance::claim_in(directory.path(), sender, true)
        .map_err(|error| io::Error::other(error.to_string()))?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "first process should own the test instance",
            )
        })?;
    let (second_sender, _second_receiver) = mpsc::sync_channel(1);
    assert!(
        PrimaryInstance::claim_in(directory.path(), second_sender, false)
            .map_err(|error| io::Error::other(error.to_string()))?
            .is_none()
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    drop(instance);
    Ok(())
}
