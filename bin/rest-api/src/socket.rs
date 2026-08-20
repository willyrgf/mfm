use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener as StdUnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};

const UNIX_PATH_MAX: usize = 107;
const LOCK_SUFFIX: &[u8] = b".mfm-rest.lock";
const MARKER_PREFIX: &str = "mfm.rest.socket.v1";
const MAX_MARKER_BYTES: u64 = 256;

pub(crate) struct BoundSocket {
    listener: tokio::net::UnixListener,
    cleanup: SocketCleanup,
}

impl BoundSocket {
    pub(crate) fn bind(path: &Path, recover_stale: bool) -> io::Result<Self> {
        validate_socket_path(path)?;
        let (parent, leaf) = open_parent(path)?;
        validate_parent(&parent)?;
        let leaf_path = child_path(&parent, &leaf);
        let lock_leaf = lock_leaf(&leaf)?;
        let lock = open_lock(&parent, &lock_leaf)?;
        lock.try_lock().map_err(|_| invalid())?;

        match fs::symlink_metadata(&leaf_path) {
            Ok(metadata) => {
                if !recover_stale {
                    return Err(invalid());
                }
                recover(&parent, &leaf_path, &lock, &metadata)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid()),
        }

        let listener = StdUnixListener::bind(&leaf_path).map_err(|_| invalid())?;
        fs::set_permissions(&leaf_path, fs::Permissions::from_mode(0o600))
            .map_err(|_| invalid())?;
        let metadata = fs::symlink_metadata(&leaf_path).map_err(|_| invalid())?;
        let identity = SocketIdentity::from_metadata(&metadata)?;
        write_marker(&lock, identity)?;
        parent.sync_all().map_err(|_| invalid())?;
        listener.set_nonblocking(true).map_err(|_| invalid())?;
        let listener = tokio::net::UnixListener::from_std(listener).map_err(|_| invalid())?;
        Ok(Self {
            listener,
            cleanup: SocketCleanup {
                parent,
                leaf_path,
                lock,
                identity,
            },
        })
    }

    pub(crate) fn into_parts(self) -> (tokio::net::UnixListener, SocketCleanup) {
        (self.listener, self.cleanup)
    }
}

pub(crate) struct SocketCleanup {
    parent: File,
    leaf_path: PathBuf,
    lock: File,
    identity: SocketIdentity,
}

impl SocketCleanup {
    pub(crate) fn cleanup(self) -> io::Result<()> {
        let metadata = fs::symlink_metadata(&self.leaf_path).map_err(|_| invalid())?;
        if !self.identity.matches(&metadata) {
            return Err(invalid());
        }
        fs::remove_file(&self.leaf_path).map_err(|_| invalid())?;
        self.parent.sync_all().map_err(|_| invalid())?;
        clear_marker(&self.lock)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

impl SocketIdentity {
    fn from_metadata(metadata: &fs::Metadata) -> io::Result<Self> {
        if !metadata.file_type().is_socket()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(invalid());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    fn matches(self, metadata: &fs::Metadata) -> bool {
        Self::from_metadata(metadata)
            .is_ok_and(|current| current.device == self.device && current.inode == self.inode)
    }
}

fn validate_socket_path(path: &Path) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() || bytes.len() > UNIX_PATH_MAX || bytes.contains(&0) {
        return Err(invalid());
    }
    Ok(())
}

fn open_parent(path: &Path) -> io::Result<(File, OsString)> {
    let leaf = path.file_name().ok_or_else(invalid)?.to_owned();
    if leaf.as_bytes() == b"." || leaf.as_bytes() == b".." || leaf.as_bytes().contains(&0) {
        return Err(invalid());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut directory = if parent.is_absolute() {
        open_directory(Path::new("/"))?
    } else {
        open_directory(Path::new("."))?
    };
    for component in parent.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => {
                directory = open_directory(&child_path(&directory, name))?;
            }
            Component::ParentDir | Component::Prefix(_) => return Err(invalid()),
        }
    }
    Ok((directory, leaf))
}

fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| invalid())
}

fn validate_parent(parent: &File) -> io::Result<()> {
    let metadata = parent.metadata().map_err(|_| invalid())?;
    if !metadata.is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o077 != 0
        || metadata.mode() & 0o700 != 0o700
    {
        return Err(invalid());
    }
    Ok(())
}

fn child_path(parent: &File, child: &OsStr) -> PathBuf {
    let mut path = PathBuf::from("/proc/self/fd");
    path.push(parent.as_raw_fd().to_string());
    path.push(child);
    path
}

fn lock_leaf(leaf: &OsStr) -> io::Result<OsString> {
    let mut bytes = leaf.as_bytes().to_vec();
    bytes.extend_from_slice(LOCK_SUFFIX);
    if bytes.len() > 255 || bytes.contains(&0) {
        return Err(invalid());
    }
    Ok(OsString::from_vec(bytes))
}

fn open_lock(parent: &File, leaf: &OsStr) -> io::Result<File> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(child_path(parent, leaf))
        .map_err(|_| invalid())?;
    let metadata = lock.metadata().map_err(|_| invalid())?;
    if !metadata.is_file() || metadata.uid() != effective_uid() || metadata.mode() & 0o777 != 0o600
    {
        return Err(invalid());
    }
    Ok(lock)
}

fn recover(
    parent: &File,
    leaf_path: &Path,
    lock: &File,
    metadata: &fs::Metadata,
) -> io::Result<()> {
    let retained = read_marker(lock)?;
    let current = SocketIdentity::from_metadata(metadata)?;
    if retained.device != current.device || retained.inode != current.inode {
        return Err(invalid());
    }
    match UnixStream::connect(leaf_path) {
        Ok(_) => return Err(invalid()),
        Err(error) if error.raw_os_error() == Some(libc::ECONNREFUSED) => {}
        Err(_) => return Err(invalid()),
    }
    fs::remove_file(leaf_path).map_err(|_| invalid())?;
    parent.sync_all().map_err(|_| invalid())
}

fn read_marker(lock: &File) -> io::Result<SocketIdentity> {
    let mut reader = lock.try_clone().map_err(|_| invalid())?;
    reader.seek(SeekFrom::Start(0)).map_err(|_| invalid())?;
    let mut bytes = Vec::new();
    reader
        .take(MAX_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_MARKER_BYTES {
        return Err(invalid());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let mut fields = text.split_whitespace();
    if fields.next() != Some(MARKER_PREFIX) {
        return Err(invalid());
    }
    let device = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(invalid)?;
    let inode = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or_else(invalid)?;
    if fields.next().is_some() {
        return Err(invalid());
    }
    Ok(SocketIdentity { device, inode })
}

fn write_marker(lock: &File, identity: SocketIdentity) -> io::Result<()> {
    let marker = format!("{MARKER_PREFIX} {} {}\n", identity.device, identity.inode);
    let mut writer = lock.try_clone().map_err(|_| invalid())?;
    writer.set_len(0).map_err(|_| invalid())?;
    writer.seek(SeekFrom::Start(0)).map_err(|_| invalid())?;
    writer.write_all(marker.as_bytes()).map_err(|_| invalid())?;
    writer.sync_all().map_err(|_| invalid())
}

fn clear_marker(lock: &File) -> io::Result<()> {
    lock.set_len(0).map_err(|_| invalid())?;
    lock.sync_all().map_err(|_| invalid())
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and does not dereference memory.
    unsafe { libc::geteuid() }
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "invalid REST socket")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "mfm-rest-socket-test-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory must be created");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("test directory mode must be set");
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join("mfm.sock")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("test directory must be removed");
        }
    }

    #[tokio::test]
    async fn bind_is_owner_only_and_graceful_cleanup_is_identity_checked() {
        let directory = TestDirectory::new();
        let path = directory.socket();
        let bound = BoundSocket::bind(&path, false).expect("socket must bind");
        let metadata = fs::symlink_metadata(&path).expect("socket must exist");
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.mode() & 0o777, 0o600);
        let (listener, cleanup) = bound.into_parts();
        drop(listener);
        cleanup.cleanup().expect("socket must be cleaned up");
        assert!(!path.exists());
        assert!(directory.0.join("mfm.sock.mfm-rest.lock").exists());
    }

    #[tokio::test]
    async fn active_or_unmarked_socket_and_foreign_leaf_are_never_removed() {
        let directory = TestDirectory::new();
        let path = directory.socket();
        let active = BoundSocket::bind(&path, false).expect("socket must bind");
        assert!(BoundSocket::bind(&path, true).is_err());
        let (listener, cleanup) = active.into_parts();
        drop(listener);
        cleanup.cleanup().expect("socket must be cleaned up");

        fs::write(&path, b"foreign").expect("foreign leaf must be created");
        assert!(BoundSocket::bind(&path, true).is_err());
        assert_eq!(
            fs::read(&path).expect("foreign leaf must remain"),
            b"foreign"
        );
    }

    #[tokio::test]
    async fn explicit_recovery_removes_only_the_recorded_refused_socket() {
        let directory = TestDirectory::new();
        let path = directory.socket();
        let crashed = BoundSocket::bind(&path, false).expect("socket must bind");
        let (listener, cleanup) = crashed.into_parts();
        drop(listener);
        drop(cleanup);

        assert!(BoundSocket::bind(&path, false).is_err());
        let recovered = BoundSocket::bind(&path, true).expect("recorded stale socket must recover");
        let (listener, cleanup) = recovered.into_parts();
        drop(listener);
        cleanup.cleanup().expect("recovered socket must clean up");
    }

    #[tokio::test]
    async fn mismatched_markers_inodes_and_cleanup_replacements_are_refused() {
        let directory = TestDirectory::new();
        let path = directory.socket();
        let crashed = BoundSocket::bind(&path, false).expect("socket must bind");
        let (listener, cleanup) = crashed.into_parts();
        drop(listener);
        drop(cleanup);
        fs::write(directory.0.join("mfm.sock.mfm-rest.lock"), b"torn").expect("marker must change");
        assert!(BoundSocket::bind(&path, true).is_err());
        assert!(fs::symlink_metadata(&path)
            .expect("socket must remain")
            .file_type()
            .is_socket());

        fs::remove_file(&path).expect("stale socket must be removed by the test");
        let replacement = StdUnixListener::bind(&path).expect("replacement socket must bind");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode must be set");
        assert!(BoundSocket::bind(&path, true).is_err());
        assert!(path.exists());
        drop(replacement);

        fs::remove_file(&path).expect("replacement socket must be removed by the test");
        fs::write(directory.0.join("mfm.sock.mfm-rest.lock"), b"").expect("marker must clear");
        let bound = BoundSocket::bind(&path, false).expect("socket must bind again");
        let (listener, cleanup) = bound.into_parts();
        drop(listener);
        fs::remove_file(&path).expect("socket must be replaced by the test");
        fs::write(&path, b"foreign").expect("foreign replacement must be created");
        assert!(cleanup.cleanup().is_err());
        assert_eq!(
            fs::read(&path).expect("replacement must remain"),
            b"foreign"
        );
    }

    #[test]
    fn unsafe_parent_modes_and_symlinked_components_are_rejected() {
        let directory = TestDirectory::new();
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o750))
            .expect("mode must change");
        assert!(BoundSocket::bind(&directory.socket(), false).is_err());

        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700))
            .expect("mode must change");
        let real = directory.0.join("real");
        fs::create_dir(&real).expect("directory must be created");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).expect("mode must change");
        let link = directory.0.join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink must be created");
        assert!(BoundSocket::bind(&link.join("mfm.sock"), false).is_err());
    }
}
