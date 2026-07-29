use crate::command::normalize_path;
use maestria_ports::PortError;
use std::path::{Path, PathBuf};

pub(super) fn fd_identity(file: &std::fs::File) -> Result<PathBuf, PortError> {
    use std::os::fd::AsRawFd;

    let link =
        std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map_err(|error| {
            PortError::InternalContext {
                context: "verify harness opened file identity",
                source: error.to_string(),
            }
        })?;
    let identity = link.to_string_lossy();
    if identity.ends_with(" (deleted)") || !link.is_absolute() {
        return Err(PortError::InternalContext {
            context: "verify harness opened file identity",
            source: format!("opened file has unstable identity {}", link.display()),
        });
    }
    Ok(normalize_path(&link))
}

pub(super) fn open_beneath(
    root: &std::fs::File,
    relative: &Path,
) -> Result<std::fs::File, std::io::Error> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    const RESOLVE_BENEATH: u64 = 0x08;
    const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
    let path = if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    };
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let how = OpenHow {
        flags: (libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NONBLOCK) as u64,
        mode: 0,
        resolve: RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS,
    };
    // SAFETY: pointers refer to live, immutable values for the duration of
    // the syscall; the returned descriptor is owned immediately by File.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            root.as_raw_fd(),
            path.as_ptr(),
            &how as *const OpenHow,
            std::mem::size_of::<OpenHow>(),
        ) as libc::c_int
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a newly returned descriptor and ownership transfers here.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

pub(super) fn ensure_regular_file(file: &std::fs::File) -> Result<(), std::io::Error> {
    use std::mem::MaybeUninit;
    use std::os::fd::AsRawFd;

    let mut stat = MaybeUninit::<libc::stat>::uninit();
    // SAFETY: `stat` points to writable storage for the duration of fstat.
    let result = unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fstat initialized `stat` on success.
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Err(std::io::Error::other("unsupported file type"));
    }
    Ok(())
}
