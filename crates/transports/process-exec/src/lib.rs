#![warn(missing_docs)]
//! Shared hardened process execution helpers for live IO transports.
//!
//! This crate contains the bounded child-process runner used by live transport crates that need
//! local process execution. It intentionally depends only on Tokio and standard library process
//! types, keeping concrete process behavior out of the state-machine contract crate.
//!
//! # Examples
//!
//! ```rust
//! use mfm_transports_process_exec::StreamLimit;
//!
//! let limits = StreamLimit {
//!     max_stdout_bytes: 1024,
//!     max_stderr_bytes: 1024,
//! };
//! assert_eq!(limits.max_stdout_bytes, 1024);
//! ```
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::time::{timeout_at, Instant};

const READ_CHUNK_SIZE: usize = 8 * 1024;

/// Output limits applied while collecting child stdout and stderr.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamLimit {
    /// Maximum number of stdout bytes retained in memory before truncation is reported.
    pub max_stdout_bytes: usize,
    /// Maximum number of stderr bytes retained in memory before truncation is reported.
    pub max_stderr_bytes: usize,
}

/// Bounded capture of a single child output stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectedStream {
    /// Retained prefix of the stream, bounded by the configured stream limit.
    pub bytes: Vec<u8>,
    /// Total number of bytes read from the child stream, including truncated bytes.
    pub total_bytes: usize,
    /// Whether additional bytes were observed after the retained prefix filled the limit.
    pub overflowed: bool,
}

/// Reduced IO error information captured while interacting with child stdin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StdinWriteError {
    /// `std::io::ErrorKind` captured while writing to or closing child stdin.
    pub kind: std::io::ErrorKind,
}

/// Result of a completed child-process execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRunResult {
    /// Final exit status returned by the child process.
    pub status: ExitStatus,
    /// Bounded stdout capture collected during execution.
    pub stdout: CollectedStream,
    /// Bounded stderr capture collected during execution.
    pub stderr: CollectedStream,
    /// Error returned while writing stdin, if the write failed after spawn succeeded.
    pub stdin_write_error: Option<StdinWriteError>,
    /// Error returned while closing stdin, if shutdown failed after writing.
    pub stdin_close_error: Option<StdinWriteError>,
}

/// Failure modes returned by [`run_command`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessRunError {
    /// Spawning the child process failed before execution began.
    SpawnFailed,
    /// The command exceeded the supplied deadline and was killed.
    Timeout,
    /// Waiting for process termination failed after spawn succeeded.
    WaitFailed,
    /// Reading stdout failed or the stdout capture task terminated unexpectedly.
    StdoutReadFailed,
    /// Reading stderr failed or the stderr capture task terminated unexpectedly.
    StderrReadFailed,
}

/// Runs a child process with bounded stdout/stderr capture and deadline enforcement.
///
/// The child is configured with piped stdin/stdout/stderr, killed on drop, and reaped on timeout.
/// Stdin write or close failures are surfaced in [`ProcessRunResult`] when the process otherwise
/// runs to completion.
pub async fn run_command(
    mut cmd: Command,
    stdin_bytes: Option<Vec<u8>>,
    timeout: Duration,
    limits: StreamLimit,
) -> Result<ProcessRunResult, ProcessRunError> {
    cmd.kill_on_drop(true);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|_| ProcessRunError::SpawnFailed)?;
    let deadline = Instant::now() + timeout;

    let stdout_handle = child
        .stdout
        .take()
        .ok_or(ProcessRunError::StdoutReadFailed)?;
    let stderr_handle = child
        .stderr
        .take()
        .ok_or(ProcessRunError::StderrReadFailed)?;

    let stdout_task = tokio::spawn(read_stream(stdout_handle, limits.max_stdout_bytes));
    let stderr_task = tokio::spawn(read_stream(stderr_handle, limits.max_stderr_bytes));

    let (stdin_write_error, stdin_close_error) =
        write_and_close_stdin(&mut child, stdin_bytes, deadline).await?;

    let status = wait_for_exit_or_timeout(&mut child, deadline).await?;

    let stdout = join_stream_task(stdout_task, ProcessRunError::StdoutReadFailed).await?;
    let stderr = join_stream_task(stderr_task, ProcessRunError::StderrReadFailed).await?;

    Ok(ProcessRunResult {
        status,
        stdout,
        stderr,
        stdin_write_error,
        stdin_close_error,
    })
}

async fn write_and_close_stdin(
    child: &mut Child,
    stdin_bytes: Option<Vec<u8>>,
    deadline: Instant,
) -> Result<(Option<StdinWriteError>, Option<StdinWriteError>), ProcessRunError> {
    let Some(mut stdin) = child.stdin.take() else {
        return Ok((None, None));
    };

    let mut write_error = None;
    let mut close_error = None;

    if let Some(bytes) = stdin_bytes {
        if !bytes.is_empty() {
            match timeout_at(deadline, stdin.write_all(&bytes)).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    write_error = Some(StdinWriteError { kind: err.kind() });
                }
                Err(_) => {
                    kill_and_reap(child).await;
                    return Err(ProcessRunError::Timeout);
                }
            }
        }
    }

    match timeout_at(deadline, stdin.shutdown()).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => {
            close_error = Some(StdinWriteError { kind: err.kind() });
        }
        Err(_) => {
            kill_and_reap(child).await;
            return Err(ProcessRunError::Timeout);
        }
    }

    Ok((write_error, close_error))
}

async fn wait_for_exit_or_timeout(
    child: &mut Child,
    deadline: Instant,
) -> Result<ExitStatus, ProcessRunError> {
    match timeout_at(deadline, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(_)) => Err(ProcessRunError::WaitFailed),
        Err(_) => {
            kill_and_reap(child).await;
            Err(ProcessRunError::Timeout)
        }
    }
}

async fn kill_and_reap(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn join_stream_task(
    handle: tokio::task::JoinHandle<Result<CollectedStream, std::io::Error>>,
    map_err: ProcessRunError,
) -> Result<CollectedStream, ProcessRunError> {
    let joined = handle.await.map_err(|_| map_err)?;
    joined.map_err(|_| map_err)
}

async fn read_stream<R>(mut reader: R, max_bytes: usize) -> Result<CollectedStream, std::io::Error>
where
    R: AsyncRead + Unpin,
{
    let mut collected = Vec::new();
    let mut total_bytes = 0usize;
    let mut overflowed = false;
    let mut chunk = [0u8; READ_CHUNK_SIZE];

    loop {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            break;
        }

        total_bytes = total_bytes.saturating_add(n);
        let remaining = max_bytes.saturating_sub(collected.len());
        if remaining > 0 {
            let take = remaining.min(n);
            collected.extend_from_slice(&chunk[..take]);
            if take < n {
                overflowed = true;
            }
        } else {
            overflowed = true;
        }
    }

    Ok(CollectedStream {
        bytes: collected,
        total_bytes,
        overflowed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn write_test_program(script_body: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("mfm-process-exec-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let program = root.join("app.sh");
        std::fs::write(&program, format!("#!/bin/sh\n{script_body}\n")).expect("write script");
        #[cfg(unix)]
        {
            let mut perms = std::fs::metadata(&program).expect("metadata").permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&program, perms).expect("chmod");
        }
        program
    }

    #[tokio::test]
    async fn timeout_covers_stdin_write() {
        let program = write_test_program("while :; do :; done");

        let cmd = Command::new(&program);
        let stdin = vec![b'a'; 4 * 1024 * 1024];
        let err = run_command(
            cmd,
            Some(stdin),
            Duration::from_millis(20),
            StreamLimit {
                max_stdout_bytes: 256,
                max_stderr_bytes: 256,
            },
        )
        .await
        .expect_err("expected timeout");

        assert_eq!(err, ProcessRunError::Timeout);

        std::fs::remove_file(&program).expect("cleanup program");
        std::fs::remove_dir_all(program.parent().expect("parent")).expect("cleanup dir");
    }

    #[tokio::test]
    async fn stream_overflow_is_detected_without_unbounded_growth() {
        let payload = "12345678".repeat(256);
        let program = write_test_program(&format!("printf '{payload}'"));
        let cmd = Command::new(&program);
        let out = run_command(
            cmd,
            None,
            Duration::from_secs(5),
            StreamLimit {
                max_stdout_bytes: 1024,
                max_stderr_bytes: 1024,
            },
        )
        .await
        .expect("run should succeed");

        assert!(out.status.success());
        assert!(out.stdout.overflowed);
        assert_eq!(out.stdout.bytes.len(), 1024);
        assert!(out.stdout.total_bytes >= 1024);

        std::fs::remove_file(&program).expect("cleanup program");
        std::fs::remove_dir_all(program.parent().expect("parent")).expect("cleanup dir");
    }
}
