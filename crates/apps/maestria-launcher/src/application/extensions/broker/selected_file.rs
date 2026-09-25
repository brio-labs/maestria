use std::fmt;
use std::fs::File;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(target_os = "linux")]
use std::fs::OpenOptions;
#[cfg(target_os = "linux")]
use std::os::unix::fs::{FileExt, OpenOptionsExt};

/// A host-authorized selection backed by an already-open file descriptor.
/// Extension input supplies only the opaque ID installed by the host.
#[derive(Clone)]
pub struct SelectedFile {
    file: Arc<File>,
}

impl fmt::Debug for SelectedFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SelectedFile([host-authorized])")
    }
}

impl SelectedFile {
    /// Captures a native file-chooser selection without trusting later path lookups.
    pub fn from_host_selection(path: PathBuf) -> io::Result<Self> {
        if !path.is_absolute() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "host selection must be an absolute local path",
            ));
        }
        let path = path.canonicalize()?;
        let file = open_readonly(&path)?;
        if !file.metadata()?.is_file() {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "host selection must refer to a regular file",
            ));
        }
        Ok(Self {
            file: Arc::new(file),
        })
    }

    pub(super) fn descriptor(&self) -> &File {
        self.file.as_ref()
    }

    pub(super) fn read_text(&self, max_bytes: usize) -> io::Result<(String, bool)> {
        if !(1..=16_384).contains(&max_bytes) {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "selected-file byte bound is invalid",
            ));
        }
        #[cfg(target_os = "linux")]
        {
            let mut bytes = vec![0; max_bytes + 1];
            let mut length = 0;
            while length < bytes.len() {
                let offset = u64::try_from(length).map_err(|_| {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        "file offset exceeds platform bound",
                    )
                })?;
                let read = self.file.read_at(&mut bytes[length..], offset)?;
                if read == 0 {
                    break;
                }
                length += read;
            }
            bytes.truncate(length);
            let already_truncated = length > max_bytes;
            super::text::decode_bounded(bytes, max_bytes, already_truncated).map_err(|_| {
                io::Error::new(ErrorKind::InvalidData, "selected file is not UTF-8 text")
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = max_bytes;
            Err(io::Error::new(
                ErrorKind::Unsupported,
                "safe selected-file reads are unavailable on this platform",
            ))
        }
    }
}

#[cfg(target_os = "linux")]
fn open_readonly(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    options.open(path)
}

#[cfg(not(target_os = "linux"))]
fn open_readonly(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        ErrorKind::Unsupported,
        "safe selected-file access is unavailable on this platform",
    ))
}
