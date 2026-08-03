//! PostgreSQL wire proxy for deterministic commit-acknowledgement qualification.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinSet;
use url::Url;

use crate::ProviderTestError;

const MAX_POSTGRES_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// One deterministic fault applied to the next PostgreSQL `COMMIT` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFault {
    /// Close both directions before forwarding `COMMIT`, proving rollback and key absence.
    RollBackBeforeCommit,
    /// Commit upstream, consume its acknowledgement, and close before the client observes it.
    CommitAndLoseAcknowledgement,
    /// Fail the client acknowledgement while retaining the live upstream transaction until released.
    HoldTransactionBeforeCommit,
}

/// A test-only TCP proxy that injects bounded PostgreSQL commit acknowledgement faults.
pub struct PostgresCommitFaultProxy {
    database_url: String,
    state: Arc<ProxyState>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl PostgresCommitFaultProxy {
    /// Starts a loopback proxy for the supplied PostgreSQL URL.
    pub async fn start(database_url: &str) -> Result<Self, ProviderTestError> {
        let mut upstream_url = Url::parse(database_url).map_err(|_| ProviderTestError::Invalid)?;
        let upstream_host = upstream_url
            .host_str()
            .ok_or(ProviderTestError::Invalid)?
            .to_owned();
        let upstream_port = upstream_url.port().unwrap_or(5432);
        let mut upstream_addresses =
            tokio::net::lookup_host((upstream_host.as_str(), upstream_port))
                .await
                .map_err(|_| ProviderTestError::Unavailable)?;
        let upstream = upstream_addresses
            .next()
            .ok_or(ProviderTestError::Unavailable)?;
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
            .await
            .map_err(|_| ProviderTestError::Unavailable)?;
        let endpoint = listener
            .local_addr()
            .map_err(|_| ProviderTestError::Unavailable)?;
        upstream_url
            .set_host(Some(&endpoint.ip().to_string()))
            .map_err(|_| ProviderTestError::Invalid)?;
        upstream_url
            .set_port(Some(endpoint.port()))
            .map_err(|_| ProviderTestError::Invalid)?;

        let state = Arc::new(ProxyState::default());
        let server_state = Arc::clone(&state);
        let (shutdown, mut shutdown_requested) = oneshot::channel();
        tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else {
                            break;
                        };
                        let connection_state = Arc::clone(&server_state);
                        connections.spawn(async move {
                            let _ = proxy_connection(client, upstream, connection_state).await;
                        });
                    }
                    _ = &mut shutdown_requested => {
                        connections.abort_all();
                        while connections.join_next().await.is_some() {}
                        break;
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });

        Ok(Self {
            database_url: upstream_url.to_string(),
            state,
            shutdown: Some(shutdown),
        })
    }

    /// Returns the PostgreSQL URL routed through this proxy.
    pub fn database_url(&self) -> &str {
        &self.database_url
    }

    /// Arms a fault for the next `attempts` commit messages and returns the intercept target.
    pub fn arm(&self, fault: CommitFault, attempts: usize) -> Result<u64, ProviderTestError> {
        if attempts == 0 {
            return Err(ProviderTestError::Invalid);
        }
        let mut plan = self
            .state
            .plan
            .lock()
            .map_err(|_| ProviderTestError::Unavailable)?;
        if plan.is_some() {
            return Err(ProviderTestError::Invalid);
        }
        let attempts = u64::try_from(attempts).map_err(|_| ProviderTestError::Invalid)?;
        let target = self
            .state
            .intercepted
            .load(Ordering::Acquire)
            .checked_add(attempts)
            .ok_or(ProviderTestError::Invalid)?;
        *plan = Some(FaultPlan {
            fault,
            remaining: attempts,
        });
        Ok(target)
    }

    /// Waits until the intercept count reaches the target returned by [`Self::arm`].
    pub async fn wait_for_intercepts(&self, target: u64) {
        loop {
            let notified = self.state.intercepted_changed.notified();
            if self.state.intercepted.load(Ordering::Acquire) >= target {
                return;
            }
            notified.await;
        }
    }

    /// Releases all upstream transactions retained by `HoldTransactionBeforeCommit`.
    pub fn release_held_transactions(&self) {
        self.state.release_generation.fetch_add(1, Ordering::AcqRel);
        self.state.release_changed.notify_waiters();
    }
}

impl Drop for PostgresCommitFaultProxy {
    fn drop(&mut self) {
        self.release_held_transactions();
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[derive(Default)]
struct ProxyState {
    plan: Mutex<Option<FaultPlan>>,
    intercepted: AtomicU64,
    intercepted_changed: Notify,
    release_generation: AtomicU64,
    release_changed: Notify,
}

struct FaultPlan {
    fault: CommitFault,
    remaining: u64,
}

impl ProxyState {
    fn take_fault(&self) -> io::Result<Option<CommitFault>> {
        let fault = {
            let mut retained = self
                .plan
                .lock()
                .map_err(|_| io::Error::other("commit fault plan unavailable"))?;
            let Some(plan) = retained.as_mut() else {
                return Ok(None);
            };
            let fault = plan.fault;
            plan.remaining = plan.remaining.saturating_sub(1);
            if plan.remaining == 0 {
                *retained = None;
            }
            fault
        };
        self.intercepted.fetch_add(1, Ordering::AcqRel);
        self.intercepted_changed.notify_one();
        Ok(Some(fault))
    }

    async fn wait_for_release(&self, retained_generation: u64) {
        loop {
            let notified = self.release_changed.notified();
            if self.release_generation.load(Ordering::Acquire) != retained_generation {
                return;
            }
            notified.await;
        }
    }
}

async fn proxy_connection(
    mut client: TcpStream,
    upstream: SocketAddr,
    state: Arc<ProxyState>,
) -> io::Result<()> {
    let mut server = TcpStream::connect(upstream).await?;
    let mut startup_forwarded = false;
    let mut suppress_commit_acknowledgement = false;
    loop {
        tokio::select! {
            frontend = read_frontend_frame(&mut client, startup_forwarded) => {
                let frontend = frontend?;
                startup_forwarded = true;
                if frontend.is_commit() {
                    match state.take_fault()? {
                        Some(CommitFault::RollBackBeforeCommit) => {
                            server.shutdown().await?;
                            client.shutdown().await?;
                            return Ok(());
                        }
                        Some(CommitFault::CommitAndLoseAcknowledgement) => {
                            server.write_all(&frontend.bytes).await?;
                            suppress_commit_acknowledgement = true;
                        }
                        Some(CommitFault::HoldTransactionBeforeCommit) => {
                            let retained_generation =
                                state.release_generation.load(Ordering::Acquire);
                            client.shutdown().await?;
                            state.wait_for_release(retained_generation).await;
                            return Ok(());
                        }
                        None => server.write_all(&frontend.bytes).await?,
                    }
                } else {
                    server.write_all(&frontend.bytes).await?;
                }
            }
            backend = read_backend_frame(&mut server) => {
                let backend = backend?;
                if suppress_commit_acknowledgement {
                    if backend.kind() == b'Z' {
                        client.shutdown().await?;
                        return Ok(());
                    }
                } else {
                    client.write_all(&backend.bytes).await?;
                }
            }
        }
    }
}

struct PostgresFrame {
    bytes: Vec<u8>,
    typed: bool,
}

impl PostgresFrame {
    fn kind(&self) -> u8 {
        if self.typed {
            self.bytes[0]
        } else {
            u8::default()
        }
    }

    fn is_commit(&self) -> bool {
        if !self.typed || self.kind() != b'Q' || self.bytes.len() <= 5 {
            return false;
        }
        std::str::from_utf8(&self.bytes[5..])
            .ok()
            .map(|query| query.trim_end_matches('\0').trim())
            .is_some_and(|query| query.eq_ignore_ascii_case("commit"))
    }
}

async fn read_frontend_frame(
    stream: &mut TcpStream,
    startup_forwarded: bool,
) -> io::Result<PostgresFrame> {
    if startup_forwarded {
        read_typed_frame(stream).await
    } else {
        let mut length = [0_u8; 4];
        stream.read_exact(&mut length).await?;
        let frame_length = bounded_frame_length(length)?;
        let mut bytes = Vec::with_capacity(frame_length);
        bytes.extend_from_slice(&length);
        bytes.resize(frame_length, 0);
        stream.read_exact(&mut bytes[4..]).await?;
        Ok(PostgresFrame {
            bytes,
            typed: false,
        })
    }
}

async fn read_backend_frame(stream: &mut TcpStream) -> io::Result<PostgresFrame> {
    read_typed_frame(stream).await
}

async fn read_typed_frame(stream: &mut TcpStream) -> io::Result<PostgresFrame> {
    let mut kind = [0_u8; 1];
    stream.read_exact(&mut kind).await?;
    let mut length = [0_u8; 4];
    stream.read_exact(&mut length).await?;
    let body_length = bounded_frame_length(length)? - 4;
    let mut bytes = Vec::with_capacity(body_length + 5);
    bytes.push(kind[0]);
    bytes.extend_from_slice(&length);
    bytes.resize(body_length + 5, 0);
    stream.read_exact(&mut bytes[5..]).await?;
    Ok(PostgresFrame { bytes, typed: true })
}

fn bounded_frame_length(bytes: [u8; 4]) -> io::Result<usize> {
    let length = usize::try_from(u32::from_be_bytes(bytes))
        .map_err(|_| io::Error::other("invalid PostgreSQL frame length"))?;
    if !(4..=MAX_POSTGRES_FRAME_BYTES).contains(&length) {
        return Err(io::Error::other("invalid PostgreSQL frame length"));
    }
    Ok(length)
}
