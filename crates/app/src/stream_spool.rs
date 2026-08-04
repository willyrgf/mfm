use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncSeekExt, AsyncWrite, AsyncWriteExt, ReadBuf, SeekFrom};

/// Private writable scratch with no path-bearing API.
pub(crate) struct WritableSpool {
    file: tokio::fs::File,
}

impl WritableSpool {
    pub(crate) async fn create() -> io::Result<Self> {
        let file = tokio::task::spawn_blocking(tempfile::tempfile)
            .await
            .map_err(|_| io::Error::other("private spool creation task failed"))??;
        Ok(Self::from_std(file))
    }

    fn from_std(file: std::fs::File) -> Self {
        Self {
            file: tokio::fs::File::from_std(file),
        }
    }

    /// Consumes writable and seekable scratch, then exposes only a reader fixed at byte zero.
    pub(crate) async fn finish(mut self) -> io::Result<FinalizedSpool> {
        self.flush().await?;
        self.file.seek(SeekFrom::Start(0)).await?;
        Ok(FinalizedSpool { file: self.file })
    }
}

impl AsyncWrite for WritableSpool {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.file).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.file).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.file).poll_shutdown(context)
    }
}

/// Private read-only view of one flushed immutable snapshot.
pub(crate) struct FinalizedSpool {
    file: tokio::fs::File,
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
