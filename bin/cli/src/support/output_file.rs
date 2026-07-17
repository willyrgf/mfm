#![allow(clippy::disallowed_methods)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use uuid::Uuid;

/// Failure classes from atomically publishing a new local output file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateNewFileError {
    /// The final target already exists.
    TargetExists,
    /// The target cannot safely name a new regular file.
    InvalidPath,
    /// Complete bytes could not be durably published.
    WriteFailed,
}

/// Writes complete bytes through a same-directory temporary file and installs them without
/// replacing an existing target.
pub(crate) fn create_new_atomic(path: &Path, bytes: &[u8]) -> Result<(), CreateNewFileError> {
    create_new_atomic_with(path, |file| file.write_all(bytes))
}

fn create_new_atomic_with(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), CreateNewFileError> {
    let parent = validated_parent(path)?;
    reject_existing_target(path)?;
    let temp_path = temporary_path(path, &parent)?;

    let result = write_temporary_file(&temp_path, write)
        .and_then(|()| install_without_replacement(&temp_path, path))
        .and_then(|()| sync_parent_directory(&parent));

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn validated_parent(path: &Path) -> Result<PathBuf, CreateNewFileError> {
    let parent = path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent).map_err(|_| CreateNewFileError::InvalidPath)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(CreateNewFileError::InvalidPath);
    }

    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode();
        if (mode & 0o002) != 0 && (mode & 0o1000) == 0 {
            return Err(CreateNewFileError::InvalidPath);
        }
    }

    Ok(parent.to_path_buf())
}

fn reject_existing_target(path: &Path) -> Result<(), CreateNewFileError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CreateNewFileError::TargetExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CreateNewFileError::InvalidPath),
    }
}

fn temporary_path(path: &Path, parent: &Path) -> Result<PathBuf, CreateNewFileError> {
    let file_name = path.file_name().ok_or(CreateNewFileError::InvalidPath)?;
    Ok(parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        Uuid::new_v4()
    )))
}

fn write_temporary_file(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), CreateNewFileError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut file = options
        .open(path)
        .map_err(|_| CreateNewFileError::InvalidPath)?;
    write(&mut file).map_err(|_| CreateNewFileError::WriteFailed)?;
    file.sync_all().map_err(|_| CreateNewFileError::WriteFailed)
}

fn install_without_replacement(temp_path: &Path, path: &Path) -> Result<(), CreateNewFileError> {
    fs::hard_link(temp_path, path).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            CreateNewFileError::TargetExists
        } else {
            CreateNewFileError::WriteFailed
        }
    })?;
    fs::remove_file(temp_path).map_err(|_| CreateNewFileError::WriteFailed)
}

fn sync_parent_directory(parent: &Path) -> Result<(), CreateNewFileError> {
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| CreateNewFileError::WriteFailed)?;

    let _ = parent;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use tempfile::TempDir;

    use super::{create_new_atomic, create_new_atomic_with, CreateNewFileError};

    #[test]
    fn partial_temporary_write_failure_never_publishes_final_path() {
        let directory = TempDir::new().expect("temporary output directory");
        let path = directory.path().join("configuration.json");

        let result = create_new_atomic_with(&path, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        });

        assert_eq!(result, Err(CreateNewFileError::WriteFailed));
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("read temporary directory")
                .count(),
            0,
            "failed publication must remove its temporary file"
        );
    }

    #[test]
    fn complete_output_is_installed_without_replacing_existing_path() {
        let directory = TempDir::new().expect("temporary output directory");
        let path = directory.path().join("configuration.json");

        create_new_atomic(&path, b"first").expect("publish first output");
        assert_eq!(
            create_new_atomic(&path, b"second"),
            Err(CreateNewFileError::TargetExists)
        );
        assert_eq!(std::fs::read(&path).expect("read output"), b"first");
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("read temporary directory")
                .count(),
            1,
            "successful publication must not retain a temporary file"
        );
    }
}
