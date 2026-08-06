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
    /// Commit upstream, retain the consumed acknowledgement until released, then close the client.
    CommitAndHoldLostAcknowledgement,
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

    /// Returns the number of frontend statement executions observed by the proxy.
    ///
    /// The counter includes simple-query (`Q`) and extended-protocol execute (`E`)
    /// messages. It is intended for qualification tests that compare a fixed
    /// operation before and after adding historical rows.
    pub fn statement_count(&self) -> u64 {
        self.state.executed_statements.load(Ordering::Acquire)
    }

    /// Returns SQL texts observed in simple-query and extended-protocol parse messages.
    ///
    /// Tests use this only to assert that a bounded operation did not issue a
    /// lifetime aggregate over immutable history.
    pub fn statement_texts(&self) -> Vec<String> {
        self.state
            .statement_texts
            .lock()
            .map(|texts| texts.clone())
            .unwrap_or_default()
    }

    /// Arms a fault for the next `attempts` commit messages and returns the intercept target.
    pub fn arm(&self, fault: CommitFault, attempts: usize) -> Result<u64, ProviderTestError> {
        self.arm_plan(fault, attempts, None)
    }

    /// Arms a fault for the next commit after a frontend statement contains `statement`.
    ///
    /// The statement gate lets tests target one semantic transaction when a client performs
    /// several unrelated wallet mutations during recovery startup.
    pub fn arm_after_statement(
        &self,
        fault: CommitFault,
        attempts: usize,
        statement: &str,
    ) -> Result<u64, ProviderTestError> {
        let statement = statement.trim();
        if statement.is_empty() {
            return Err(ProviderTestError::Invalid);
        }
        self.arm_plan(fault, attempts, Some(statement.to_ascii_lowercase()))
    }

    fn arm_plan(
        &self,
        fault: CommitFault,
        attempts: usize,
        required_statement: Option<String>,
    ) -> Result<u64, ProviderTestError> {
        if attempts == 0 {
            return Err(ProviderTestError::Invalid);
        }
        let statement_triggered = required_statement.as_ref().is_none_or(|needle| {
            self.state
                .statement_texts
                .lock()
                .map(|texts| texts.iter().any(|statement| statement.contains(needle)))
                .unwrap_or(false)
        });
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
            triggered: statement_triggered,
            required_statement,
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

    /// Arms one committed acknowledgement loss that remains held until explicitly released.
    pub fn arm_held_lost_acknowledgement(&self) -> Result<u64, ProviderTestError> {
        let target = self
            .state
            .held_lost_acknowledgements
            .load(Ordering::Acquire)
            .checked_add(1)
            .ok_or(ProviderTestError::Invalid)?;
        self.arm(CommitFault::CommitAndHoldLostAcknowledgement, 1)?;
        Ok(target)
    }

    /// Waits until the upstream commit is durable and its client acknowledgement is held.
    pub async fn wait_for_held_lost_acknowledgements(&self, target: u64) {
        loop {
            let notified = self.state.held_lost_acknowledgements_changed.notified();
            if self
                .state
                .held_lost_acknowledgements
                .load(Ordering::Acquire)
                >= target
            {
                return;
            }
            notified.await;
        }
    }

    /// Releases retained transactions and committed acknowledgements held by this proxy.
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
    executed_statements: AtomicU64,
    statement_texts: Mutex<Vec<String>>,
    intercepted_changed: Notify,
    held_lost_acknowledgements: AtomicU64,
    held_lost_acknowledgements_changed: Notify,
    release_generation: AtomicU64,
    release_changed: Notify,
}

struct FaultPlan {
    fault: CommitFault,
    remaining: u64,
    required_statement: Option<String>,
    triggered: bool,
}

impl ProxyState {
    fn record_statement(&self, statement: &str) {
        if let Ok(mut texts) = self.statement_texts.lock() {
            texts.push(statement.to_owned());
        }
        if let Ok(mut plan) = self.plan.lock() {
            if plan.as_ref().is_some_and(|plan| {
                !plan.triggered
                    && plan
                        .required_statement
                        .as_ref()
                        .is_some_and(|needle| statement.to_ascii_lowercase().contains(needle))
            }) {
                if let Some(plan) = plan.as_mut() {
                    plan.triggered = true;
                }
            }
        }
    }

    fn take_fault(&self) -> io::Result<Option<CommitFault>> {
        let fault = {
            let mut retained = self
                .plan
                .lock()
                .map_err(|_| io::Error::other("commit fault plan unavailable"))?;
            let Some(plan) = retained.as_mut() else {
                return Ok(None);
            };
            if plan.required_statement.is_some() && !plan.triggered {
                return Ok(None);
            }
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

    fn record_held_lost_acknowledgement(&self) {
        self.held_lost_acknowledgements
            .fetch_add(1, Ordering::AcqRel);
        self.held_lost_acknowledgements_changed.notify_waiters();
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
    let mut held_acknowledgement_generation = None;
    let mut commit_completed = false;
    loop {
        tokio::select! {
            frontend = read_frontend_frame(&mut client, startup_forwarded) => {
                let frontend = frontend?;
                startup_forwarded = true;
                if frontend.typed && matches!(frontend.kind(), b'Q' | b'E') {
                    state.executed_statements.fetch_add(1, Ordering::AcqRel);
                }
                if let Some(statement) = frontend.statement_text() {
                    state.record_statement(statement);
                }
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
                        Some(CommitFault::CommitAndHoldLostAcknowledgement) => {
                            let retained_generation =
                                state.release_generation.load(Ordering::Acquire);
                            server.write_all(&frontend.bytes).await?;
                            suppress_commit_acknowledgement = true;
                            held_acknowledgement_generation = Some(retained_generation);
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
                    if backend.is_successful_commit_completion() {
                        commit_completed = true;
                    }
                    if backend.kind() == b'Z' {
                        if let Some(retained_generation) = held_acknowledgement_generation {
                            if commit_completed {
                                state.record_held_lost_acknowledgement();
                                state.wait_for_release(retained_generation).await;
                            }
                        }
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

    fn is_successful_commit_completion(&self) -> bool {
        self.typed && self.kind() == b'C' && self.bytes.get(5..) == Some(b"COMMIT\0")
    }

    fn statement_text(&self) -> Option<&str> {
        if !self.typed {
            return None;
        }
        let body = self.bytes.get(5..)?;
        let query = match self.kind() {
            b'Q' => body.split(|byte| *byte == 0).next()?,
            b'P' => {
                let name_end = body.iter().position(|byte| *byte == 0)?;
                body.get(name_end.saturating_add(1)..)?
                    .split(|byte| *byte == 0)
                    .next()?
            }
            _ => return None,
        };
        std::str::from_utf8(query).ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn backend_frame(kind: u8, body: &[u8]) -> PostgresFrame {
        let length = u32::try_from(body.len() + 4)
            .expect("bounded test frame")
            .to_be_bytes();
        let mut bytes = Vec::with_capacity(body.len() + 5);
        bytes.push(kind);
        bytes.extend_from_slice(&length);
        bytes.extend_from_slice(body);
        PostgresFrame { bytes, typed: true }
    }

    #[test]
    fn held_acknowledgement_requires_successful_commit_completion() {
        assert!(backend_frame(b'C', b"COMMIT\0").is_successful_commit_completion());
        assert!(!backend_frame(b'C', b"ROLLBACK\0").is_successful_commit_completion());
        assert!(!backend_frame(b'E', b"COMMIT\0").is_successful_commit_completion());
        assert!(!backend_frame(b'Z', b"I").is_successful_commit_completion());
    }

    #[test]
    fn statement_gated_fault_waits_for_its_transaction_marker() {
        let state = ProxyState::default();
        state
            .plan
            .lock()
            .expect("fault plan lock")
            .replace(FaultPlan {
                fault: CommitFault::HoldTransactionBeforeCommit,
                remaining: 1,
                required_statement: Some("insert into wallet_nonce_completions".to_owned()),
                triggered: false,
            });

        assert_eq!(
            state.take_fault().expect("untriggered plan"),
            None,
            "unrelated commits must pass before the marker"
        );
        state.record_statement("UPDATE wallet_nonce_domains SET current_resource_frontier_ref");
        assert_eq!(
            state.take_fault().expect("unrelated statement"),
            None,
            "unrelated statements must not arm the fault"
        );
        state.record_statement("INSERT INTO wallet_nonce_completions (...)");
        assert_eq!(
            state.take_fault().expect("triggered plan"),
            Some(CommitFault::HoldTransactionBeforeCommit)
        );
    }
}
