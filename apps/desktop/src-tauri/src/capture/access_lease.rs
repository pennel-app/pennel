//! QA-only access-token delivery from the active Capture session to an owned FIFO.

use std::collections::HashSet;
use std::ffi::{CString, OsStr};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::Shared;

const CAPTURE_ID: &str = "dev.pennel.Pennel.capture.preview";
const OPT_IN: &str = "PENNEL_CAPTURE_CLOUD_ACCESS_LEASE";
const DEADLINE: Duration = Duration::from_secs(15);
const MAX_LEASE_BYTES: usize = 64 * 1024;

fn enabled(identifier: &str, capture: bool, value: Option<&OsStr>) -> bool {
    identifier == CAPTURE_ID && capture && value == Some(OsStr::new("1"))
}

fn valid_fifo_name(name: &str) -> bool {
    name.strip_prefix("review253-access-")
        .and_then(|name| name.strip_suffix(".fifo"))
        .is_some_and(|nonce| {
            nonce.len() == 32
                && nonce
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "The owned evaluation FIFO is unavailable.",
    )
}

fn timed_out() -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        "The evaluation access lease deadline expired.",
    )
}

fn private_fifo(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_fifo()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o7777 == 0o600
}

fn open_fifo(directory: &Path, name: &str, deadline: Instant) -> io::Result<File> {
    if !valid_fifo_name(name) {
        return Err(invalid());
    }
    let parent = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory)?;
    let metadata = parent.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
        return Err(invalid());
    }
    let before = std::fs::symlink_metadata(directory.join(name))?;
    if !private_fifo(&before) || before.nlink() != 1 {
        return Err(invalid());
    }
    let name = CString::new(name).map_err(|_| invalid())?;
    loop {
        if Instant::now() >= deadline {
            return Err(timed_out());
        }
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor >= 0 {
            let file = unsafe { File::from_raw_fd(descriptor) };
            let current = file.metadata()?;
            if !private_fifo(&current)
                || before.dev() != current.dev()
                || before.ino() != current.ino()
            {
                return Err(invalid());
            }
            // A reader disappearing must become a bounded write error, never SIGPIPE in the holder.
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETNOSIGPIPE, 1) } == -1 {
                return Err(invalid());
            }
            return Ok(file);
        }
        let issue = io::Error::last_os_error();
        if issue.raw_os_error() != Some(libc::ENXIO) {
            return Err(issue);
        }
        std::thread::sleep(
            Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

struct ActiveFifo((u64, u64));

fn active_fifos() -> &'static Mutex<HashSet<(u64, u64)>> {
    static ACTIVE: OnceLock<Mutex<HashSet<(u64, u64)>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}

impl ActiveFifo {
    fn begin(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        let identity = (metadata.dev(), metadata.ino());
        if !active_fifos().lock().unwrap().insert(identity) {
            return Err(invalid());
        }
        Ok(Self(identity))
    }
}

impl Drop for ActiveFifo {
    fn drop(&mut self) {
        active_fifos().lock().unwrap().remove(&self.0);
    }
}

struct DeadlineWriter {
    file: File,
    deadline: Instant,
    written: usize,
}

impl Write for DeadlineWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.written.saturating_add(bytes.len()) > MAX_LEASE_BYTES {
            return Err(invalid());
        }
        loop {
            if Instant::now() >= self.deadline {
                return Err(timed_out());
            }
            match self.file.write(bytes) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "The evaluation reader closed.",
                    ));
                }
                Ok(count) => {
                    self.written += count;
                    return Ok(count);
                }
                Err(issue) if issue.kind() == io::ErrorKind::Interrupted => continue,
                Err(issue) if issue.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(
                    Duration::from_millis(5)
                        .min(self.deadline.saturating_duration_since(Instant::now())),
                ),
                Err(issue) => return Err(issue),
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Serialize)]
pub struct CaptureAccessLeaseView {
    written: bool,
    origin: String,
    expires_at_unix_seconds: u64,
    bytes_written: usize,
}

#[tauri::command]
pub async fn capture_cloud_access_lease(
    expected_account_epoch: String,
    fifo_name: String,
    app: AppHandle,
    shared: State<'_, Arc<Shared>>,
) -> Result<CaptureAccessLeaseView, String> {
    if !enabled(
        &app.config().identifier,
        super::enabled(&app),
        std::env::var_os(OPT_IN).as_deref(),
    ) || !shared.secrets.is_capture_memory_store()
    {
        return Err(
            "Evaluation access requires the explicitly enabled isolated Capture app.".into(),
        );
    }
    let directory =
        super::data_directory(&app)?.ok_or("The owned Capture directory is unavailable.")?;
    if directory != shared.data_dir
        || expected_account_epoch != crate::cloud::account_epoch(&shared)
    {
        return Err(
            "The Capture account or directory changed. Check the owned session again.".into(),
        );
    }
    let shared = shared.inner().clone();
    let deadline = Instant::now() + DEADLINE;
    tauri::async_runtime::spawn_blocking(move || {
        let file = open_fifo(&directory, &fifo_name, deadline)
            .map_err(|_| "The owned evaluation FIFO could not be opened.".to_string())?;
        let _active = ActiveFifo::begin(&file)
            .map_err(|_| "This evaluation FIFO is already in use.".to_string())?;
        if expected_account_epoch != crate::cloud::account_epoch(&shared) {
            return Err("The Capture account changed before leasing access.".into());
        }
        let lease = shared
            .engine
            .evaluation_cloud_access_token_lease_before(120, deadline)
            .map_err(|_| {
                "The Capture access lease could not be prepared within its deadline.".to_string()
            })?;
        if Instant::now() >= deadline
            || expected_account_epoch != crate::cloud::account_epoch(&shared)
            || !private_fifo(
                &file
                    .metadata()
                    .map_err(|_| "The owned evaluation FIFO is unavailable.".to_string())?,
            )
        {
            return Err("The Capture access lease expired or changed before delivery.".into());
        }
        let mut writer = DeadlineWriter {
            file,
            deadline,
            written: 0,
        };
        lease
            .write_secret(&mut writer)
            .map_err(|_| "The Capture access lease could not be delivered.".to_string())?;
        Ok(CaptureAccessLeaseView {
            written: true,
            origin: lease.origin().into(),
            expires_at_unix_seconds: lease.expires_at_unix_seconds(),
            bytes_written: writer.written,
        })
    })
    .await
    .map_err(|_| "The Capture access lease worker stopped.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const NAME: &str = "review253-access-11111111111111111111111111111111.fifo";

    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        directory
    }

    fn fifo(root: &Path, mode: u32) {
        let path = CString::new(root.join(NAME).as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(
            unsafe { libc::mkfifo(path.as_ptr(), mode.try_into().unwrap()) },
            0
        );
        std::fs::set_permissions(root.join(NAME), std::fs::Permissions::from_mode(mode)).unwrap();
    }
    fn reader(root: &Path) -> File {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(root.join(NAME))
            .unwrap()
    }

    #[test]
    fn ordinary_apps_and_implicit_capture_cannot_enable_access_leases() {
        assert!(enabled(CAPTURE_ID, true, Some(OsStr::new("1"))));
        for id in [
            "dev.pennel.Pennel",
            "dev.pennel.Pennel.preview",
            "another.app",
        ] {
            assert!(!enabled(id, true, Some(OsStr::new("1"))));
        }
        assert!(!enabled(CAPTURE_ID, false, Some(OsStr::new("1"))));
        for flag in [None, Some(OsStr::new("true")), Some(OsStr::new("0"))] {
            assert!(!enabled(CAPTURE_ID, true, flag));
        }
    }

    #[test]
    fn fifo_name_is_a_flat_owned_nonce_only() {
        assert!(valid_fifo_name(NAME));
        for name in [
            "",
            "../outside.fifo",
            "/tmp/outside.fifo",
            "child/review253-access-11111111111111111111111111111111.fifo",
            "review253-access-1.fifo",
            "review253-access-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA.fifo",
        ] {
            assert!(!valid_fifo_name(name));
        }
    }

    #[test]
    fn regular_files_symlinks_and_shared_permissions_are_refused() {
        let root = private_directory();
        std::fs::write(root.path().join(NAME), b"owned-original").unwrap();
        assert!(open_fifo(root.path(), NAME, Instant::now() + DEADLINE).is_err());
        assert_eq!(
            std::fs::read(root.path().join(NAME)).unwrap(),
            b"owned-original"
        );
        std::fs::remove_file(root.path().join(NAME)).unwrap();
        std::fs::write(root.path().join("target"), b"owned-target").unwrap();
        symlink(root.path().join("target"), root.path().join(NAME)).unwrap();
        assert!(open_fifo(root.path(), NAME, Instant::now() + DEADLINE).is_err());
        std::fs::remove_file(root.path().join(NAME)).unwrap();
        fifo(root.path(), 0o640);
        assert!(open_fifo(root.path(), NAME, Instant::now() + DEADLINE).is_err());
    }

    #[test]
    fn private_fifo_delivers_only_to_its_reader_and_duplicate_writers_are_rejected() {
        let root = private_directory();
        fifo(root.path(), 0o600);
        let mut input = reader(root.path());
        let file = open_fifo(root.path(), NAME, Instant::now() + DEADLINE).unwrap();
        let first = ActiveFifo::begin(&file).unwrap();
        assert!(ActiveFifo::begin(&file).is_err());
        let mut writer = DeadlineWriter {
            file,
            deadline: Instant::now() + DEADLINE,
            written: 0,
        };
        writer.write_all(b"owned-synthetic-access").unwrap();
        drop(writer);
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"owned-synthetic-access");
        drop(first);
    }

    #[test]
    fn no_reader_and_backpressure_stop_at_the_absolute_deadline() {
        let root = private_directory();
        fifo(root.path(), 0o600);
        assert_eq!(
            open_fifo(
                root.path(),
                NAME,
                Instant::now() + Duration::from_millis(15)
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::TimedOut
        );
        let _reader = reader(root.path());
        let file = open_fifo(root.path(), NAME, Instant::now() + DEADLINE).unwrap();
        let mut writer = DeadlineWriter {
            file,
            deadline: Instant::now(),
            written: 0,
        };
        assert_eq!(
            writer.write(b"owned-synthetic-access").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        let mut writer = DeadlineWriter {
            file: writer.file,
            deadline: Instant::now() + DEADLINE,
            written: MAX_LEASE_BYTES,
        };
        assert_eq!(
            writer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn shared_or_symlinked_capture_directories_are_refused() {
        let root = private_directory();
        fifo(root.path(), 0o600);
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(open_fifo(root.path(), NAME, Instant::now() + DEADLINE).is_err());
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let alias = root.path().join("alias");
        symlink(root.path(), &alias).unwrap();
        assert!(open_fifo(&alias, NAME, Instant::now() + DEADLINE).is_err());
    }

    #[test]
    fn a_slow_reader_cannot_extend_the_write_deadline() {
        let root = private_directory();
        fifo(root.path(), 0o600);
        let _reader = reader(root.path());
        let mut file = open_fifo(root.path(), NAME, Instant::now() + DEADLINE).unwrap();
        let mut filled = 0;
        loop {
            match file.write(&[0u8; 4096]) {
                Ok(count) => {
                    filled += count;
                    assert!(filled <= 1024 * 1024);
                }
                Err(issue) if issue.kind() == io::ErrorKind::WouldBlock => break,
                result => panic!("Unexpected owned FIFO fill result: {result:?}"),
            }
        }
        let started = Instant::now();
        let mut writer = DeadlineWriter {
            file,
            deadline: started + Duration::from_millis(25),
            written: 0,
        };
        assert_eq!(
            writer.write(b"owned-synthetic-access").unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn a_closed_reader_is_a_write_error_without_process_signals() {
        let root = private_directory();
        fifo(root.path(), 0o600);
        let input = reader(root.path());
        let file = open_fifo(root.path(), NAME, Instant::now() + DEADLINE).unwrap();
        drop(input);
        let mut writer = DeadlineWriter {
            file,
            deadline: Instant::now() + DEADLINE,
            written: 0,
        };
        assert_eq!(
            writer.write(b"owned-synthetic-access").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }
}
