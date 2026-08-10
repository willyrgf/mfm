#![allow(clippy::disallowed_methods)]

use std::fs::{self, File};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;
use tokio::io::{AsyncRead, AsyncWriteExt};

/// Failure classes from atomically publishing new local output files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreateNewFileError {
    /// A final target already exists.
    TargetExists,
    /// A target cannot safely name a new regular file.
    InvalidPath,
    /// Complete bytes could not be copied or durably published.
    WriteFailed,
}

/// Validates both non-overwriting targets without creating scratch or final files.
pub(crate) fn preflight_new_atomic_pair(
    first_path: &Path,
    second_path: &Path,
) -> Result<(), CreateNewFileError> {
    let first_parent = validated_parent(first_path)?;
    let second_parent = validated_parent(second_path)?;
    if resolved_target_identity(first_path, &first_parent)?
        == resolved_target_identity(second_path, &second_parent)?
    {
        return Err(CreateNewFileError::InvalidPath);
    }
    validate_new_target(first_path)?;
    validate_new_target(second_path)
}

/// Copies one stream and one bounded sidecar through same-directory temporary files.
///
/// Each final path appears only after its complete temporary file has been flushed and synced.
/// A race that creates either target is never overwritten.
pub(crate) async fn create_new_atomic_pair_from_reader<R>(
    first_path: PathBuf,
    mut first_reader: R,
    second_path: PathBuf,
    second_bytes: Vec<u8>,
) -> Result<(), CreateNewFileError>
where
    R: AsyncRead + Unpin,
{
    let (prepared, first_file, second_file) = tokio::task::spawn_blocking({
        let first_path = first_path.clone();
        let second_path = second_path.clone();
        move || PreparedPair::create(first_path, second_path)
    })
    .await
    .map_err(|_| CreateNewFileError::WriteFailed)??;

    let mut first_file = tokio::fs::File::from_std(first_file);
    let mut second_file = tokio::fs::File::from_std(second_file);
    tokio::io::copy(&mut first_reader, &mut first_file)
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)?;
    first_file
        .flush()
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)?;
    first_file
        .sync_all()
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)?;
    second_file
        .write_all(&second_bytes)
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)?;
    second_file
        .flush()
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)?;
    second_file
        .sync_all()
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)?;
    drop(first_file);
    drop(second_file);

    let published = tokio::task::spawn_blocking(move || prepared.publish())
        .await
        .map_err(|_| CreateNewFileError::WriteFailed)??;
    published.commit();
    Ok(())
}

struct PreparedPair {
    first: NamedTempFile,
    first_target: PathBuf,
    first_parent: PathBuf,
    second: NamedTempFile,
    second_target: PathBuf,
    second_parent: PathBuf,
}

impl PreparedPair {
    fn create(
        first_target: PathBuf,
        second_target: PathBuf,
    ) -> Result<(Self, File, File), CreateNewFileError> {
        preflight_new_atomic_pair(&first_target, &second_target)?;
        let first_parent = validated_parent(&first_target)?;
        let second_parent = validated_parent(&second_target)?;
        let first =
            NamedTempFile::new_in(&first_parent).map_err(|_| CreateNewFileError::WriteFailed)?;
        let second =
            NamedTempFile::new_in(&second_parent).map_err(|_| CreateNewFileError::WriteFailed)?;
        let first_file = first
            .reopen()
            .map_err(|_| CreateNewFileError::WriteFailed)?;
        let second_file = second
            .reopen()
            .map_err(|_| CreateNewFileError::WriteFailed)?;
        Ok((
            Self {
                first,
                first_target,
                first_parent,
                second,
                second_target,
                second_parent,
            },
            first_file,
            second_file,
        ))
    }

    fn publish(self) -> Result<PublishedPair, CreateNewFileError> {
        self.publish_inner(|_| {})
    }

    #[cfg(test)]
    fn publish_with_hook(
        self,
        after_sync: impl FnOnce(&mut PublishedPair),
    ) -> Result<PublishedPair, CreateNewFileError> {
        self.publish_inner(after_sync)
    }

    fn publish_inner(
        self,
        after_sync: impl FnOnce(&mut PublishedPair),
    ) -> Result<PublishedPair, CreateNewFileError> {
        let Self {
            first,
            first_target,
            first_parent,
            second,
            second_target,
            second_parent,
        } = self;
        let shared_parent = resolved_directory_identity(&first_parent)?
            == resolved_directory_identity(&second_parent)?;
        let first_file = first
            .persist_noclobber(&first_target)
            .map_err(|error| persist_error(error.error))?;
        let mut published = PublishedPair {
            first_file: Some(first_file),
            first_target,
            first_parent,
            second_file: None,
            second_target,
            second_parent,
            shared_parent,
            armed: true,
            #[cfg(test)]
            rollback_observer: None,
        };
        let second_file = match second.persist_noclobber(&published.second_target) {
            Ok(file) => file,
            Err(error) => {
                let publication_error = persist_error(error.error);
                published.rollback()?;
                return Err(publication_error);
            }
        };
        published.second_file = Some(second_file);
        if let Err(sync_error) = published.sync_parents() {
            published.rollback()?;
            return Err(sync_error);
        }
        after_sync(&mut published);
        Ok(published)
    }
}

struct PublishedPair {
    first_file: Option<File>,
    first_target: PathBuf,
    first_parent: PathBuf,
    second_file: Option<File>,
    second_target: PathBuf,
    second_parent: PathBuf,
    shared_parent: bool,
    armed: bool,
    #[cfg(test)]
    rollback_observer: Option<tokio::sync::oneshot::Sender<Result<(), CreateNewFileError>>>,
}

impl PublishedPair {
    fn commit(mut self) {
        self.armed = false;
    }

    fn sync_parents(&self) -> Result<(), CreateNewFileError> {
        sync_parent_directory(&self.first_parent)?;
        if !self.shared_parent {
            sync_parent_directory(&self.second_parent)?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), CreateNewFileError> {
        let mut failed = false;
        if let Some(file) = self.second_file.as_ref() {
            if remove_persisted_if_owned(&self.second_target, file).is_ok() {
                self.second_file = None;
            } else {
                failed = true;
            }
        }
        if let Some(file) = self.first_file.as_ref() {
            if remove_persisted_if_owned(&self.first_target, file).is_ok() {
                self.first_file = None;
            } else {
                failed = true;
            }
        }
        if self.sync_parents().is_err() {
            failed = true;
        }
        if failed {
            Err(CreateNewFileError::WriteFailed)
        } else {
            self.armed = false;
            Ok(())
        }
    }

    #[cfg(test)]
    fn observe_rollback(
        &mut self,
        observer: tokio::sync::oneshot::Sender<Result<(), CreateNewFileError>>,
    ) {
        self.rollback_observer = Some(observer);
    }
}

impl Drop for PublishedPair {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        #[cfg(test)]
        let observer = self.rollback_observer.take();
        let result = self.rollback();
        #[cfg(not(test))]
        let _ = result;
        #[cfg(test)]
        if let Some(observer) = observer {
            let _ = observer.send(result);
        }
    }
}

fn remove_persisted_if_owned(path: &Path, file: &File) -> Result<(), CreateNewFileError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let open = file
            .metadata()
            .map_err(|_| CreateNewFileError::WriteFailed)?;
        let installed = fs::symlink_metadata(path).map_err(|_| CreateNewFileError::WriteFailed)?;
        if !installed.file_type().is_file()
            || open.dev() != installed.dev()
            || open.ino() != installed.ino()
        {
            return Err(CreateNewFileError::WriteFailed);
        }
    }
    fs::remove_file(path).map_err(|_| CreateNewFileError::WriteFailed)
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

fn validate_new_target(path: &Path) -> Result<(), CreateNewFileError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CreateNewFileError::TargetExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CreateNewFileError::InvalidPath),
    }
}

fn resolved_target_identity(path: &Path, parent: &Path) -> Result<PathBuf, CreateNewFileError> {
    let file_name = path.file_name().ok_or(CreateNewFileError::InvalidPath)?;
    resolved_directory_identity(parent).map(|resolved_parent| resolved_parent.join(file_name))
}

fn resolved_directory_identity(path: &Path) -> Result<PathBuf, CreateNewFileError> {
    fs::canonicalize(path).map_err(|_| CreateNewFileError::InvalidPath)
}

fn persist_error(error: io::Error) -> CreateNewFileError {
    if error.kind() == io::ErrorKind::AlreadyExists {
        CreateNewFileError::TargetExists
    } else {
        CreateNewFileError::WriteFailed
    }
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
    use std::io;
    use std::pin::Pin;
    use std::sync::mpsc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tempfile::TempDir;
    use tokio::io::{AsyncRead, ReadBuf};
    use tokio::sync::oneshot;

    use super::{
        create_new_atomic_pair_from_reader, preflight_new_atomic_pair, CreateNewFileError,
        PreparedPair,
    };

    struct PartialFailure {
        emitted: bool,
    }

    impl AsyncRead for PartialFailure {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if self.emitted {
                return Poll::Ready(Err(io::Error::other("injected read failure")));
            }
            self.emitted = true;
            buffer.put_slice(b"partial");
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn partial_copy_failure_never_publishes_final_paths() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("run.export");
        let content_ref = directory.path().join("run.export.ref");
        let result = create_new_atomic_pair_from_reader(
            export.clone(),
            PartialFailure { emitted: false },
            content_ref.clone(),
            b"ref".to_vec(),
        )
        .await;

        assert_eq!(result, Err(CreateNewFileError::WriteFailed));
        assert!(!export.exists());
        assert!(!content_ref.exists());
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("temporary directory")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn cancellation_during_blocking_publication_rolls_back_both_parents() {
        let first_directory = TempDir::new().expect("first output directory");
        let second_directory = TempDir::new().expect("second output directory");
        let export = first_directory.path().join("run.export");
        let content_ref = second_directory.path().join("run.export.ref");
        let (prepared, first_file, second_file) =
            PreparedPair::create(export.clone(), content_ref.clone()).expect("prepared pair");
        drop(first_file);
        drop(second_file);

        let (published_sender, published_receiver) = oneshot::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (rollback_sender, rollback_receiver) = oneshot::channel();
        let publication = tokio::spawn(async move {
            let published = tokio::task::spawn_blocking(move || {
                prepared.publish_with_hook(move |published| {
                    published.observe_rollback(rollback_sender);
                    published_sender
                        .send(())
                        .expect("publication observer remains available");
                    release_receiver
                        .recv()
                        .expect("publication release remains available");
                })
            })
            .await
            .map_err(|_| CreateNewFileError::WriteFailed)??;
            published.commit();
            Ok::<(), CreateNewFileError>(())
        });

        published_receiver
            .await
            .expect("both finals durably published before cancellation");
        assert!(export.exists());
        assert!(content_ref.exists());
        publication.abort();
        assert!(publication
            .await
            .expect_err("publication future must be cancelled")
            .is_cancelled());
        release_sender
            .send(())
            .expect("detached blocking publication remains active");

        // The observer fires only after ownership-checked removal and both
        // distinct parent-directory syncs complete.
        let rollback = tokio::time::timeout(Duration::from_secs(5), rollback_receiver)
            .await
            .expect("rollback completion timeout")
            .expect("rollback observer");
        assert_eq!(rollback, Ok(()));
        assert!(!export.exists());
        assert!(!content_ref.exists());
        assert_eq!(
            std::fs::read_dir(first_directory.path())
                .expect("first output directory")
                .count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(second_directory.path())
                .expect("second output directory")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn pair_is_complete_and_never_overwrites_existing_targets() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("run.export");
        let content_ref = directory.path().join("run.export.ref");
        create_new_atomic_pair_from_reader(
            export.clone(),
            std::io::Cursor::new(b"stream".to_vec()),
            content_ref.clone(),
            b"ref".to_vec(),
        )
        .await
        .expect("paired publication");
        assert_eq!(std::fs::read(&export).expect("stream"), b"stream");
        assert_eq!(std::fs::read(&content_ref).expect("sidecar"), b"ref");
        assert_eq!(
            create_new_atomic_pair_from_reader(
                export.clone(),
                std::io::Cursor::new(b"replacement".to_vec()),
                content_ref.clone(),
                b"replacement-ref".to_vec(),
            )
            .await,
            Err(CreateNewFileError::TargetExists)
        );
        assert_eq!(std::fs::read(&export).expect("stream"), b"stream");
        assert_eq!(std::fs::read(&content_ref).expect("sidecar"), b"ref");
    }

    #[tokio::test]
    async fn streamed_output_has_no_old_sixteen_mib_total_cap() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("large.export");
        let content_ref = directory.path().join("large.export.ref");
        let bytes = vec![b'x'; 16_777_216 + 1];
        create_new_atomic_pair_from_reader(
            export.clone(),
            std::io::Cursor::new(bytes.clone()),
            content_ref,
            b"ref".to_vec(),
        )
        .await
        .expect("large streamed publication");
        assert_eq!(std::fs::read(export).expect("large output"), bytes);
    }

    #[tokio::test]
    async fn published_stream_bytes_match_the_canonical_sidecar_digest() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("run.export");
        let sidecar = directory.path().join("run.export.ref");
        let bytes = b"\x1e{\"kind\":\"end\"}\n".to_vec();
        // This suite publishes bytes and a sidecar reference; the portable
        // stream identity itself is owned and tested by `mfm-replay`.
        let content_ref = mfm_ids::ContentRef::new(
            mfm_ids::SchemaId::new(
                "mfm.test.export-stream",
                "1",
                mfm_ids::DigestAlgorithm::Sha256JcsV1,
                mfm_canonical::sha256_digest_bytes(b"mfm.test.export-stream"),
            )
            .expect("stream schema"),
            mfm_canonical::raw_content_digest(&bytes),
        )
        .expect("content ref");
        let sidecar_bytes = serde_json::to_vec(&content_ref).expect("canonical sidecar");
        create_new_atomic_pair_from_reader(
            export.clone(),
            std::io::Cursor::new(bytes),
            sidecar.clone(),
            sidecar_bytes.clone(),
        )
        .await
        .expect("paired publication");

        let published = std::fs::read(export).expect("published stream");
        let published_ref: mfm_ids::ContentRef =
            serde_json::from_slice(&std::fs::read(sidecar).expect("published sidecar"))
                .expect("content ref");
        assert_eq!(
            serde_json::to_vec(&published_ref).expect("canonical content ref"),
            sidecar_bytes
        );
        assert_eq!(
            published_ref.content_digest(),
            &mfm_canonical::raw_content_digest(&published)
        );
        assert_eq!(published_ref.schema_id(), content_ref.schema_id());
    }

    #[test]
    fn preflight_checks_both_targets_and_rejects_aliases_without_scratch() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("run.export");
        let content_ref = directory.path().join("run.export.ref");
        std::fs::write(&content_ref, b"existing").expect("existing sidecar");
        assert_eq!(
            preflight_new_atomic_pair(&export, &content_ref),
            Err(CreateNewFileError::TargetExists)
        );
        assert!(!export.exists());
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("temporary directory")
                .count(),
            1
        );

        let aliased = directory.path().join(".").join("aliased.export");
        let direct = directory.path().join("aliased.export");
        assert_eq!(
            preflight_new_atomic_pair(&aliased, &direct),
            Err(CreateNewFileError::InvalidPath)
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let real_parent = directory.path().join("real-parent");
            std::fs::create_dir(&real_parent).expect("real parent");
            let linked_parent = directory.path().join("linked-parent");
            symlink(&real_parent, &linked_parent).expect("linked parent");
            assert_eq!(
                preflight_new_atomic_pair(
                    &linked_parent.join("stream"),
                    &real_parent.join("sidecar"),
                ),
                Err(CreateNewFileError::InvalidPath)
            );

            let target = directory.path().join("target");
            symlink(&content_ref, &target).expect("target symlink");
            assert_eq!(
                preflight_new_atomic_pair(&target, &real_parent.join("other-sidecar")),
                Err(CreateNewFileError::TargetExists)
            );
        }
    }

    #[test]
    fn second_target_race_rolls_back_the_first_final_and_all_scratch() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("run.export");
        let content_ref = directory.path().join("run.export.ref");
        let (prepared, _first, _second) =
            PreparedPair::create(export.clone(), content_ref.clone()).expect("prepared pair");
        std::fs::write(&content_ref, b"racing sidecar").expect("racing target");
        assert!(matches!(
            prepared.publish(),
            Err(CreateNewFileError::TargetExists)
        ));
        assert!(!export.exists());
        assert_eq!(
            std::fs::read(&content_ref).expect("racing target"),
            b"racing sidecar"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("temporary directory")
                .count(),
            1
        );
    }

    #[test]
    fn first_target_race_publishes_neither_file_and_removes_all_scratch() {
        let directory = TempDir::new().expect("temporary output directory");
        let export = directory.path().join("run.export");
        let content_ref = directory.path().join("run.export.ref");
        let (prepared, _first, _second) =
            PreparedPair::create(export.clone(), content_ref.clone()).expect("prepared pair");
        std::fs::write(&export, b"racing export").expect("racing target");
        assert!(matches!(
            prepared.publish(),
            Err(CreateNewFileError::TargetExists)
        ));
        assert_eq!(
            std::fs::read(&export).expect("racing target"),
            b"racing export"
        );
        assert!(!content_ref.exists());
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("temporary directory")
                .count(),
            1
        );
    }

    #[test]
    fn prepared_scratch_is_secure_and_in_each_target_directory() {
        let first_directory = TempDir::new().expect("first output directory");
        let second_directory = TempDir::new().expect("second output directory");
        let (prepared, _first, _second) = PreparedPair::create(
            first_directory.path().join("run.export"),
            second_directory.path().join("run.export.ref"),
        )
        .expect("prepared pair");
        assert_eq!(prepared.first.path().parent(), Some(first_directory.path()));
        assert_eq!(
            prepared.second.path().parent(),
            Some(second_directory.path())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                prepared
                    .first
                    .as_file()
                    .metadata()
                    .expect("first metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                prepared
                    .second
                    .as_file()
                    .metadata()
                    .expect("second metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
