use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use mfm_ids::ContentRef;
use tokio::io::{AsyncRead, AsyncSeekExt, AsyncWrite, AsyncWriteExt, ReadBuf, SeekFrom};

use crate::ExportStreamInput;

/// Private writable scratch with no path-bearing API.
pub(crate) struct WritableSpool {
    file: tokio::fs::File,
    #[cfg(test)]
    failure: Option<TestFailure>,
    #[cfg(test)]
    lifetime: Option<TestLifetime>,
}

impl WritableSpool {
    pub(crate) async fn create() -> io::Result<Self> {
        let file = tokio::task::spawn_blocking(tempfile::tempfile)
            .await
            .map_err(|_| io::Error::other("private spool creation task failed"))??;
        Ok(Self::from_std(file))
    }

    #[cfg(test)]
    async fn create_in(directory: std::path::PathBuf) -> io::Result<Self> {
        let file = tokio::task::spawn_blocking(move || tempfile::tempfile_in(directory))
            .await
            .map_err(|_| io::Error::other("private spool creation task failed"))??;
        Ok(Self::from_std(file))
    }

    fn from_std(file: std::fs::File) -> Self {
        Self {
            file: tokio::fs::File::from_std(file),
            #[cfg(test)]
            failure: None,
            #[cfg(test)]
            lifetime: None,
        }
    }

    /// Consumes writable and seekable scratch, then exposes only a reader fixed at byte zero.
    pub(crate) async fn finish(mut self) -> io::Result<FinalizedSpool> {
        self.flush().await?;
        #[cfg(test)]
        if self.failure == Some(TestFailure::Rewind) {
            return Err(test_error());
        }
        self.file.seek(SeekFrom::Start(0)).await?;
        Ok(FinalizedSpool {
            file: self.file,
            #[cfg(test)]
            _lifetime: self.lifetime.take(),
        })
    }
}

impl AsyncWrite for WritableSpool {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        #[cfg(test)]
        if self.failure == Some(TestFailure::Write) {
            return Poll::Ready(Err(test_error()));
        }
        Pin::new(&mut self.file).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        #[cfg(test)]
        if self.failure == Some(TestFailure::Flush) {
            return Poll::Ready(Err(test_error()));
        }
        Pin::new(&mut self.file).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.file).poll_shutdown(context)
    }
}

/// Private read-only view of one flushed immutable snapshot.
pub(crate) struct FinalizedSpool {
    file: tokio::fs::File,
    #[cfg(test)]
    _lifetime: Option<TestLifetime>,
}

impl AsyncRead for FinalizedSpool {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.file).poll_read(context, buffer)
    }
}

/// Drains one already-authorized caller stream into stable private scratch.
pub(crate) async fn snapshot_input(
    input: ExportStreamInput,
) -> io::Result<(ContentRef, FinalizedSpool)> {
    snapshot_input_with(input, WritableSpool::create).await
}

async fn snapshot_input_with<Create, CreateFuture>(
    input: ExportStreamInput,
    create: Create,
) -> io::Result<(ContentRef, FinalizedSpool)>
where
    Create: FnOnce() -> CreateFuture,
    CreateFuture: Future<Output = io::Result<WritableSpool>>,
{
    let (content_ref, mut reader) = input.into_parts();
    let mut spool = create().await?;
    tokio::io::copy(&mut reader, &mut spool).await?;
    let spool = spool.finish().await?;
    Ok((content_ref, spool))
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestFailure {
    Write,
    Flush,
    Rewind,
}

#[cfg(test)]
fn test_error() -> io::Error {
    io::Error::other("private sentinel that must never escape")
}

#[cfg(test)]
struct TestLifetime(std::sync::Arc<std::sync::atomic::AtomicUsize>);

#[cfg(test)]
impl Drop for TestLifetime {
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll};

    use mfm_canonical::RecoverabilityContractV3;
    use mfm_ids::ContentRef;
    use static_assertions::assert_not_impl_any;
    use tokio::io::{AsyncRead, AsyncReadExt, ReadBuf};

    use super::{
        snapshot_input, snapshot_input_with, FinalizedSpool, TestFailure, TestLifetime,
        WritableSpool,
    };
    use crate::{ExportAsyncReader, ExportStreamInput};

    assert_not_impl_any!(WritableSpool: Clone);
    assert_not_impl_any!(FinalizedSpool: Clone, tokio::io::AsyncWrite, tokio::io::AsyncSeek);

    struct ProbeReader {
        bytes: &'static [u8],
        offset: usize,
        polls: Arc<AtomicUsize>,
        fail: bool,
        pending: bool,
    }

    impl AsyncRead for ProbeReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            self.polls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Poll::Ready(Err(io::Error::other(
                    "private source sentinel that must never escape",
                )));
            }
            if self.pending {
                return Poll::Pending;
            }
            let remaining = &self.bytes[self.offset..];
            let count = remaining.len().min(buffer.remaining());
            buffer.put_slice(&remaining[..count]);
            self.offset += count;
            Poll::Ready(Ok(()))
        }
    }

    fn input(reader: ExportAsyncReader) -> ExportStreamInput {
        let contract = RecoverabilityContractV3::embedded().expect("recoverability contract");
        let content_ref = ContentRef::new(
            contract
                .schema_id("mfm.portable-run-export-stream.v2")
                .expect("stream schema")
                .clone(),
            contract.raw_content_digest(b"snapshot"),
        )
        .expect("content ref");
        ExportStreamInput::from_reader(content_ref, reader).expect("stream input")
    }

    async fn injected_spool(
        failure: Option<TestFailure>,
        drops: Arc<AtomicUsize>,
    ) -> io::Result<WritableSpool> {
        let mut spool = WritableSpool::create().await?;
        spool.failure = failure;
        spool.lifetime = Some(TestLifetime(drops));
        Ok(spool)
    }

    #[tokio::test]
    async fn creation_failure_does_not_poll_the_input() {
        let polls = Arc::new(AtomicUsize::new(0));
        let input = input(Box::pin(ProbeReader {
            bytes: b"unpolled",
            offset: 0,
            polls: Arc::clone(&polls),
            fail: false,
            pending: false,
        }));
        let result = snapshot_input_with(input, || async {
            Err(io::Error::other(
                "private creation sentinel that must never escape",
            ))
        })
        .await;
        let error = match result {
            Ok(_) => panic!("creation must fail"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn copy_flush_and_rewind_failures_drop_private_scratch() {
        for failure in [
            None,
            Some(TestFailure::Write),
            Some(TestFailure::Flush),
            Some(TestFailure::Rewind),
        ] {
            let drops = Arc::new(AtomicUsize::new(0));
            let fail_source = failure.is_none();
            let input = input(Box::pin(ProbeReader {
                bytes: b"snapshot",
                offset: 0,
                polls: Arc::new(AtomicUsize::new(0)),
                fail: fail_source,
                pending: false,
            }));
            let drops_for_create = Arc::clone(&drops);
            let result =
                snapshot_input_with(input, || injected_spool(failure, drops_for_create)).await;
            assert!(result.is_err(), "{failure:?}");
            assert_eq!(drops.load(Ordering::SeqCst), 1, "{failure:?}");
        }
    }

    #[tokio::test]
    async fn cancellation_drops_the_unnamed_spool() {
        let polls = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        let input = input(Box::pin(ProbeReader {
            bytes: b"",
            offset: 0,
            polls: Arc::clone(&polls),
            fail: false,
            pending: true,
        }));
        let drops_for_create = Arc::clone(&drops);
        let task = tokio::spawn(async move {
            snapshot_input_with(input, || injected_spool(None, drops_for_create)).await
        });
        while polls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        task.abort();
        let _ = task.await;
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn finalized_snapshot_is_rewound_and_independent_of_source_mutation() {
        let directory = tempfile::TempDir::new().expect("source directory");
        let source_path = directory.path().join("source.export");
        tokio::fs::write(&source_path, b"original")
            .await
            .expect("source bytes");
        let source = tokio::fs::File::open(&source_path)
            .await
            .expect("source reader");
        let (_, mut snapshot) = snapshot_input(input(Box::pin(source)))
            .await
            .expect("finalized snapshot");

        tokio::fs::write(&source_path, b"mutated")
            .await
            .expect("mutated source");
        let mut bytes = Vec::new();
        snapshot
            .read_to_end(&mut bytes)
            .await
            .expect("snapshot bytes");
        assert_eq!(bytes, b"original");
    }

    #[tokio::test]
    async fn scratch_is_unnamed_before_and_after_finalization() {
        let directory = tempfile::TempDir::new().expect("scratch directory");
        let mut spool = WritableSpool::create_in(directory.path().to_path_buf())
            .await
            .expect("unnamed spool");
        tokio::io::AsyncWriteExt::write_all(&mut spool, b"scratch")
            .await
            .expect("scratch bytes");
        tokio::io::AsyncWriteExt::shutdown(&mut spool)
            .await
            .expect("writer shutdown");
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("scratch directory")
                .count(),
            0
        );
        let mut finalized = spool.finish().await.expect("finalized spool");
        let mut bytes = Vec::new();
        finalized
            .read_to_end(&mut bytes)
            .await
            .expect("finalized bytes");
        assert_eq!(bytes, b"scratch");
        drop(finalized);
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("scratch directory")
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn snapshot_has_no_old_sixteen_mib_total_cap() {
        let bytes = vec![b'x'; 16_777_216 + 1];
        let (_, mut snapshot) =
            snapshot_input(input(Box::pin(std::io::Cursor::new(bytes.clone()))))
                .await
                .expect("large snapshot");
        let mut copied = Vec::new();
        snapshot
            .read_to_end(&mut copied)
            .await
            .expect("large snapshot bytes");
        assert_eq!(copied, bytes);
    }
}
