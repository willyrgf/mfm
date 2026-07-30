//! Test-only prototype for capability-free historical executable reproduction.
//!
//! The prototype deliberately does not add a production replay API. It copies this integration
//! test executable as a stand-in for a retained historical artifact, pins its exact
//! `mfm.executable-bytes.v1` identity, and communicates with the copied process through one
//! versioned stdin/stdout protocol. A qualified child runs inside Bubblewrap on Linux or a
//! deny-default `sandbox-exec` profile on macOS. The child starts with a cleared environment and
//! receives no capability descriptor or live-authority handle unless a denial test deliberately
//! requests one. If the platform sandbox is missing or cannot establish its boundary, the mode is
//! `Unavailable` and no callback runs.
//!
//! Honest limitations:
//!
//! - The Linux boundary unshares user, PID, network, IPC, UTS, and mount namespaces; mounts only
//!   the retained executable and its exact dynamic-loader files read-only; provides a tmpfs
//!   scratch directory; drops capabilities; disables nested user namespaces; and verifies that no
//!   non-protocol file descriptor reached the worker. Qualification also actively rejects an
//!   outbound TCP connection and reads of `/etc/passwd` and a host-side credential sentinel before
//!   callbacks. Bubblewrap synthesizes only `PWD=/tmp` after clearing the environment. The macOS
//!   profile is deny-default and grants only the retained executable/system-loader reads and one
//!   scratch directory.
//! - This is not a complete hostile-code sandbox. The Linux worker still sees a private procfs and
//!   can use syscalls such as clocks and `getrandom`; no seccomp syscall policy is qualified here.
//!   The active probes demonstrate the named denials, not that every possible ambient resource is
//!   absent. The macOS profile depends on deprecated platform tooling and is not exercised by
//!   Linux CI.
//! - The executable and the protocol implementation are trusted test code. A child could lie in
//!   its report; production isolation needs an independently enforced boundary.
//! - Copying the current test executable proves exact-byte handshaking on Linux and macOS. It does
//!   not establish long-term artifact retention, loader compatibility, code-signing policy, or
//!   reproducibility across toolchains and operating-system versions.
//! - The structural and exact result types below remain conformance models. The test-only adapter
//!   exercises the real bytes-only `mfm-replay` resolver trait, but constructs no store/run
//!   authority and creates no parallel production replay path.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{Command, Output, Stdio};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use mfm_canonical::{CanonicalValue, RecoverabilityContract};
use mfm_ids::{ContentDigest, ContentRef, JournalCommitDigest, JournalRecordHash, RunId};
use mfm_journal::{JournalHead, RecordRef, TransitionRef};
use mfm_replay::{
    ExactReproduction as ReplayExactReproduction, ExactReproductionPlan, ReproductionFuture,
    ReproductionResolver,
};
use serde::{Deserialize, Serialize};

const PROTOCOL: &str = "mfm.replay.capability-free-executable.v1";
const EXECUTABLE_IDENTITY_CONTRACT: &str = "mfm.executable-bytes.v1";
const EXECUTABLE_DESCRIPTOR_CONTRACT: &str = "mfm.executable-bytes-descriptor.v1";
const WORKER_ENV: &str = "MFM_REPLAY_CAPABILITY_FREE_WORKER";
const RESPONSE_PREFIX: &str = "MFM_REPLAY_CAPABILITY_FREE_RESPONSE:";
#[cfg(target_os = "linux")]
const SANDBOX_EXECUTABLE_PATH: &str = "/replay/executable";
#[cfg(target_os = "linux")]
const SANDBOX_SCRATCH_PATH: &str = "/tmp";
static TEMP_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

type ExactWorkerRequestParts = (String, Vec<(String, TransitionRef)>, Vec<TransitionRequest>);

#[derive(Debug)]
struct RetainedExecutable {
    directory: TemporaryDirectory,
    path: PathBuf,
    retained_identity: String,
    ambient_credential_probe: PathBuf,
}

impl RetainedExecutable {
    fn capture() -> io::Result<Self> {
        let directory = TemporaryDirectory::new()?;
        let path = directory.path().join("retained-replay-executable");
        let ambient_credential_probe = directory.path().join("ambient-credential");
        fs::copy(std::env::current_exe()?, &path)?;
        fs::write(&ambient_credential_probe, b"test-only credential sentinel")?;
        let retained_identity = executable_identity_for_path(&path)?;
        Ok(Self {
            directory,
            path,
            retained_identity,
            ambient_credential_probe,
        })
    }

    fn append_unadmitted_bytes(&self) -> io::Result<()> {
        OpenOptions::new()
            .append(true)
            .open(&self.path)?
            .write_all(b"changed after retention")
    }

    fn remove(&self) -> io::Result<()> {
        fs::remove_file(&self.path)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn retained_identity(&self) -> &str {
        &self.retained_identity
    }

    fn ambient_credential_probe(&self) -> &Path {
        &self.ambient_credential_probe
    }

    fn directory_exists(&self) -> bool {
        self.directory.path().is_dir()
    }

    #[cfg(target_os = "macos")]
    fn scratch_path(&self) -> PathBuf {
        self.directory.path().join("scratch")
    }
}

#[derive(Debug)]
struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new() -> io::Result<Self> {
        let nonce = TEMP_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mfm-replay-isolation-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuralVerification {
    transition_count: usize,
}

fn verify_recorded_history_structurally(
    transitions: &[TransitionRequest],
) -> Result<StructuralVerification, &'static str> {
    let mut transition_refs = BTreeSet::new();
    for transition in transitions {
        if !transition_refs.insert(transition.transition_ref.as_str()) {
            return Err("duplicate transition ref");
        }
    }
    Ok(StructuralVerification {
        transition_count: transitions.len(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExactReproduction {
    Matched(ModeEvidence),
    Mismatch {
        transition_ref: String,
        evidence: ModeEvidence,
    },
    Unavailable {
        reason: UnavailableReason,
        evidence: Option<ModeEvidence>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModeEvidence {
    self_attested_identity: String,
    trace: Vec<TraceEvent>,
    callback_count: usize,
    live_authority_minted: bool,
}

impl From<&WorkerResponse> for ModeEvidence {
    fn from(response: &WorkerResponse) -> Self {
        Self {
            self_attested_identity: response.self_attested_identity.clone(),
            trace: response.trace.clone(),
            callback_count: response.callback_count,
            live_authority_minted: response.live_authority_minted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UnavailableReason {
    ArtifactChanged,
    ArtifactMissing,
    ArtifactUnreadable,
    CapabilityDescriptorDenied,
    CapabilityEnvironmentPresent,
    ExecutableIdentityMismatch,
    InheritedFileDescriptorPresent,
    LaunchFailed,
    ProtocolViolation,
    SandboxAttestationMissing,
    SandboxUnavailable,
}

#[derive(Debug, Clone)]
enum SandboxBackend {
    #[cfg(target_os = "linux")]
    LinuxBubblewrap {
        program: PathBuf,
    },
    #[cfg(target_os = "macos")]
    MacOsSandboxExec {
        program: PathBuf,
    },
    Unavailable,
}

impl SandboxBackend {
    fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            find_program("bwrap")
                .map(|program| Self::LinuxBubblewrap { program })
                .unwrap_or(Self::Unavailable)
        }
        #[cfg(target_os = "macos")]
        {
            find_program("sandbox-exec")
                .map(|program| Self::MacOsSandboxExec { program })
                .unwrap_or(Self::Unavailable)
        }
    }
}

struct PrototypeLauncher {
    backend: SandboxBackend,
    launch_count: AtomicUsize,
}

impl PrototypeLauncher {
    fn new() -> Self {
        Self {
            backend: SandboxBackend::detect(),
            launch_count: AtomicUsize::new(0),
        }
    }

    fn unavailable() -> Self {
        Self {
            backend: SandboxBackend::Unavailable,
            launch_count: AtomicUsize::new(0),
        }
    }

    fn launch_count(&self) -> usize {
        self.launch_count.load(Ordering::Relaxed)
    }

    fn reproduce_exact(
        &self,
        artifact: &RetainedExecutable,
        admitted_executable_identity: &str,
        capability_descriptors: Vec<String>,
        transitions: Vec<TransitionRequest>,
    ) -> ExactReproduction {
        if let Err(reason) = verify_retained_artifact(artifact) {
            return ExactReproduction::Unavailable {
                reason,
                evidence: None,
            };
        }
        let sandbox = match sandbox_expectation(artifact) {
            Ok(sandbox) => sandbox,
            Err(reason) => {
                return ExactReproduction::Unavailable {
                    reason,
                    evidence: None,
                };
            }
        };
        let request = WorkerRequest {
            protocol: PROTOCOL.to_owned(),
            expected_executable_identity: admitted_executable_identity.to_owned(),
            sandbox,
            capability_descriptors,
            transitions,
        };
        let response = match self.launch(artifact.path(), &request) {
            Ok(response) => response,
            Err(reason) => {
                return ExactReproduction::Unavailable {
                    reason,
                    evidence: None,
                };
            }
        };
        let evidence = ModeEvidence::from(&response);
        match response.outcome {
            WorkerOutcome::ExactMatched => ExactReproduction::Matched(evidence),
            WorkerOutcome::ExactMismatch { transition_ref } => ExactReproduction::Mismatch {
                transition_ref,
                evidence,
            },
            WorkerOutcome::Unavailable { reason } => ExactReproduction::Unavailable {
                reason,
                evidence: Some(evidence),
            },
        }
    }

    fn launch(
        &self,
        executable: &Path,
        request: &WorkerRequest,
    ) -> Result<WorkerResponse, UnavailableReason> {
        let mut command = match &self.backend {
            #[cfg(target_os = "linux")]
            SandboxBackend::LinuxBubblewrap { program } => {
                linux_bubblewrap_command(program, executable)?
            }
            #[cfg(target_os = "macos")]
            SandboxBackend::MacOsSandboxExec { program } => {
                macos_sandbox_exec_command(program, executable, &request.sandbox)?
            }
            SandboxBackend::Unavailable => return Err(UnavailableReason::SandboxUnavailable),
        };
        self.launch_count.fetch_add(1, Ordering::Relaxed);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        run_worker_command(command, request, UnavailableReason::SandboxUnavailable)
    }

    fn launch_env_clear_control(
        &self,
        executable: &Path,
        request: &WorkerRequest,
    ) -> Result<WorkerResponse, UnavailableReason> {
        let mut command = Command::new(executable);
        command
            .args(worker_test_arguments())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear()
            .env(WORKER_ENV, "1");
        run_worker_command(command, request, UnavailableReason::LaunchFailed)
    }
}

/// Test-only bridge from the frozen bytes-only replay callback to the qualified sandbox
/// prototype. It deliberately retains the executable out of band: the replay service can pass
/// only canonical plan bytes.
struct QualifiedResolverAdapter<'a> {
    launcher: &'a PrototypeLauncher,
    artifact: &'a RetainedExecutable,
}

impl ReproductionResolver for QualifiedResolverAdapter<'_> {
    fn reproduce_exact<'a>(
        &'a self,
        canonical_plan: &'a [u8],
    ) -> ReproductionFuture<'a, ReplayExactReproduction> {
        Box::pin(async move {
            let Ok((expected_identity, transition_refs, requests)) =
                exact_worker_request(canonical_plan)
            else {
                return ReplayExactReproduction::Unavailable;
            };
            match self.launcher.reproduce_exact(
                self.artifact,
                &expected_identity,
                Vec::new(),
                requests,
            ) {
                ExactReproduction::Matched(_) => ReplayExactReproduction::Matched,
                ExactReproduction::Mismatch { transition_ref, .. } => {
                    let transition_ref = transition_refs
                        .into_iter()
                        .find(|(encoded, _)| encoded == &transition_ref)
                        .map(|(_, reference)| reference);
                    match transition_ref {
                        Some(transition_ref) => ReplayExactReproduction::Mismatch {
                            transition_ref: Some(transition_ref),
                        },
                        None => ReplayExactReproduction::Unavailable,
                    }
                }
                ExactReproduction::Unavailable { .. } => ReplayExactReproduction::Unavailable,
            }
        })
    }
}

fn exact_worker_request(canonical_plan: &[u8]) -> Result<ExactWorkerRequestParts, ()> {
    let plan = ExactReproductionPlan::strict_decode(canonical_plan).map_err(|_| ())?;
    let value = plan.canonical_value().map_err(|_| ())?;
    let admitted = canonical_field(&value, "admitted_executable_identity_ref")?;
    let expected_identity = canonical_string(admitted, "content_digest")?.to_owned();
    let CanonicalValue::Array(transitions) = canonical_field(&value, "ordered_transition_refs")?
    else {
        return Err(());
    };
    let transition_refs = transitions
        .iter()
        .cloned()
        .map(|value| {
            let reference = TransitionRef::from_canonical_value(value).map_err(|_| ())?;
            let encoded = String::from_utf8(reference.as_bytes().to_vec()).map_err(|_| ())?;
            Ok((encoded, reference))
        })
        .collect::<Result<Vec<_>, ()>>()?;
    let requests = transition_refs
        .iter()
        .map(|(encoded, _)| TransitionRequest::comparable(encoded, "recorded", "recorded"))
        .collect();
    Ok((expected_identity, transition_refs, requests))
}

fn canonical_field<'a>(value: &'a CanonicalValue, field: &str) -> Result<&'a CanonicalValue, ()> {
    let CanonicalValue::Object(object) = value else {
        return Err(());
    };
    object
        .entries()
        .find_map(|(key, value)| (key == field).then_some(value))
        .ok_or(())
}

fn canonical_string<'a>(value: &'a CanonicalValue, field: &str) -> Result<&'a str, ()> {
    match canonical_field(value, field)? {
        CanonicalValue::String(value) => Ok(value),
        _ => Err(()),
    }
}

fn resolve_ready<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test resolver unexpectedly yielded"),
    }
}

fn run_worker_command(
    mut command: Command,
    request: &WorkerRequest,
    command_failure: UnavailableReason,
) -> Result<WorkerResponse, UnavailableReason> {
    let mut child = command.spawn().map_err(|_| command_failure)?;
    let request_bytes =
        serde_json::to_vec(request).map_err(|_| UnavailableReason::ProtocolViolation)?;
    child
        .stdin
        .take()
        .ok_or(UnavailableReason::ProtocolViolation)?
        .write_all(&request_bytes)
        .map_err(|_| UnavailableReason::ProtocolViolation)?;
    let output = child.wait_with_output().map_err(|_| command_failure)?;
    parse_worker_response(output, command_failure)
}

fn worker_test_arguments() -> [&'static str; 5] {
    [
        "retained_executable_worker",
        "--exact",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]
}

fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(target_os = "linux")]
fn platform_sandbox_kind() -> SandboxKind {
    SandboxKind::LinuxBubblewrap
}

#[cfg(target_os = "macos")]
fn platform_sandbox_kind() -> SandboxKind {
    SandboxKind::MacOsSandboxExec
}

#[cfg(target_os = "linux")]
fn sandbox_expectation(
    artifact: &RetainedExecutable,
) -> Result<SandboxExpectation, UnavailableReason> {
    let parent_namespaces = ["user", "pid", "net", "ipc", "uts", "mnt"]
        .into_iter()
        .map(|name| {
            let path = Path::new("/proc/self/ns").join(name);
            let identity = fs::read_link(path)
                .map_err(|_| UnavailableReason::SandboxUnavailable)?
                .to_string_lossy()
                .into_owned();
            Ok(NamespaceIdentity {
                name: name.to_owned(),
                identity,
            })
        })
        .collect::<Result<Vec<_>, UnavailableReason>>()?;
    Ok(SandboxExpectation {
        kind: SandboxKind::LinuxBubblewrap,
        expected_executable_path: SANDBOX_EXECUTABLE_PATH.to_owned(),
        scratch_path: SANDBOX_SCRATCH_PATH.to_owned(),
        ambient_probe_path: artifact
            .ambient_credential_probe()
            .to_string_lossy()
            .into_owned(),
        parent_namespaces,
    })
}

#[cfg(target_os = "linux")]
fn linux_bubblewrap_command(
    program: &Path,
    executable: &Path,
) -> Result<Command, UnavailableReason> {
    let loader_files = linux_dynamic_loader_files(executable)?;
    let mut destination_directories =
        BTreeSet::from([PathBuf::from("/nix"), PathBuf::from("/nix/store")]);
    for (_, destination) in &loader_files {
        let mut parent = destination.parent();
        while let Some(directory) = parent {
            if directory == Path::new("/nix/store") {
                break;
            }
            destination_directories.insert(directory.to_owned());
            parent = directory.parent();
        }
    }
    let mut destination_directories = destination_directories.into_iter().collect::<Vec<_>>();
    destination_directories.sort_by_key(|path| path.components().count());

    let mut command = Command::new(program);
    command
        .args([
            "--unshare-user",
            "--unshare-pid",
            "--unshare-net",
            "--unshare-ipc",
            "--unshare-uts",
            "--unshare-cgroup-try",
            "--disable-userns",
            "--die-with-parent",
            "--new-session",
            "--cap-drop",
            "ALL",
            "--clearenv",
            "--setenv",
            WORKER_ENV,
            "1",
        ])
        .env_clear();
    for directory in destination_directories {
        command.arg("--dir").arg(directory);
    }
    for (source, destination) in loader_files {
        command.arg("--ro-bind").arg(source).arg(destination);
    }
    command
        .args(["--proc", "/proc", "--tmpfs", SANDBOX_SCRATCH_PATH])
        .args(["--dir", "/replay"])
        .arg("--ro-bind")
        .arg(executable)
        .arg(SANDBOX_EXECUTABLE_PATH)
        .args(["--chdir", SANDBOX_SCRATCH_PATH, "--"])
        .arg(SANDBOX_EXECUTABLE_PATH)
        .args(worker_test_arguments());
    Ok(command)
}

#[cfg(target_os = "linux")]
fn linux_dynamic_loader_files(
    executable: &Path,
) -> Result<Vec<(PathBuf, PathBuf)>, UnavailableReason> {
    let ldd = find_program("ldd").ok_or(UnavailableReason::SandboxUnavailable)?;
    let output = Command::new(ldd)
        .arg(executable)
        .output()
        .map_err(|_| UnavailableReason::SandboxUnavailable)?;
    if !output.status.success() {
        return Err(UnavailableReason::SandboxUnavailable);
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| UnavailableReason::SandboxUnavailable)?;
    if stdout.lines().any(|line| line.contains("not found")) {
        return Err(UnavailableReason::SandboxUnavailable);
    }
    let mut files = BTreeSet::new();
    for token in stdout.split_whitespace() {
        if !token.starts_with('/') {
            continue;
        }
        let destination = PathBuf::from(token);
        if !destination.starts_with("/nix/store") || !destination.is_file() {
            return Err(UnavailableReason::SandboxUnavailable);
        }
        let source =
            fs::canonicalize(&destination).map_err(|_| UnavailableReason::SandboxUnavailable)?;
        files.insert((source, destination));
    }
    Ok(files.into_iter().collect())
}

#[cfg(target_os = "macos")]
fn sandbox_expectation(
    artifact: &RetainedExecutable,
) -> Result<SandboxExpectation, UnavailableReason> {
    let executable =
        fs::canonicalize(artifact.path()).map_err(|_| UnavailableReason::SandboxUnavailable)?;
    let scratch = artifact.scratch_path();
    fs::create_dir(&scratch).map_err(|_| UnavailableReason::SandboxUnavailable)?;
    let scratch = fs::canonicalize(scratch).map_err(|_| UnavailableReason::SandboxUnavailable)?;
    Ok(SandboxExpectation {
        kind: SandboxKind::MacOsSandboxExec,
        expected_executable_path: executable.to_string_lossy().into_owned(),
        scratch_path: scratch.to_string_lossy().into_owned(),
        ambient_probe_path: artifact
            .ambient_credential_probe()
            .to_string_lossy()
            .into_owned(),
        parent_namespaces: Vec::new(),
    })
}

#[cfg(target_os = "macos")]
fn macos_sandbox_exec_command(
    program: &Path,
    executable: &Path,
    expectation: &SandboxExpectation,
) -> Result<Command, UnavailableReason> {
    let executable =
        fs::canonicalize(executable).map_err(|_| UnavailableReason::SandboxUnavailable)?;
    let profile = format!(
        r#"(version 1)
(deny default)
(allow process*)
(allow signal (target self))
(allow sysctl-read)
(allow file-read* (literal "{executable}") (subpath "/System/Library") (subpath "/usr/lib") (subpath "/nix/store") (subpath "/dev/fd"))
(allow file-read* file-write* (subpath "{scratch}"))
"#,
        executable = sandbox_profile_literal(&executable),
        scratch = sandbox_profile_literal(Path::new(&expectation.scratch_path)),
    );
    let mut command = Command::new(program);
    command
        .args(["-p", &profile, "--"])
        .arg(executable)
        .args(worker_test_arguments())
        .current_dir(&expectation.scratch_path)
        .env_clear()
        .env(WORKER_ENV, "1");
    Ok(command)
}

#[cfg(target_os = "macos")]
fn sandbox_profile_literal(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', r"\\")
        .replace('"', r#"\""#)
}

fn verify_retained_artifact(artifact: &RetainedExecutable) -> Result<(), UnavailableReason> {
    let actual = executable_identity_for_path(artifact.path()).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            UnavailableReason::ArtifactMissing
        } else {
            UnavailableReason::ArtifactUnreadable
        }
    })?;
    if actual != artifact.retained_identity() {
        return Err(UnavailableReason::ArtifactChanged);
    }
    Ok(())
}

fn parse_worker_response(
    output: Output,
    command_failure: UnavailableReason,
) -> Result<WorkerResponse, UnavailableReason> {
    if !output.status.success() {
        return Err(command_failure);
    }
    let response = [&output.stdout[..], &output.stderr[..]]
        .into_iter()
        .find_map(|bytes| {
            let text = String::from_utf8_lossy(bytes);
            let start = text.find(RESPONSE_PREFIX)? + RESPONSE_PREFIX.len();
            let response = text[start..].lines().next()?;
            serde_json::from_str::<WorkerResponse>(response).ok()
        })
        .ok_or(UnavailableReason::ProtocolViolation)?;
    if response.protocol != PROTOCOL {
        return Err(UnavailableReason::ProtocolViolation);
    }
    Ok(response)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SandboxExpectation {
    kind: SandboxKind,
    expected_executable_path: String,
    scratch_path: String,
    ambient_probe_path: String,
    parent_namespaces: Vec<NamespaceIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SandboxKind {
    LinuxBubblewrap,
    MacOsSandboxExec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct NamespaceIdentity {
    name: String,
    identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkerRequest {
    protocol: String,
    expected_executable_identity: String,
    sandbox: SandboxExpectation,
    capability_descriptors: Vec<String>,
    transitions: Vec<TransitionRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TransitionRequest {
    transition_ref: String,
    recorded_output: String,
    callback_output: String,
}

impl TransitionRequest {
    fn comparable(transition_ref: &str, recorded_output: &str, callback_output: &str) -> Self {
        Self {
            transition_ref: transition_ref.to_owned(),
            recorded_output: recorded_output.to_owned(),
            callback_output: callback_output.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WorkerResponse {
    protocol: String,
    self_attested_identity: String,
    live_authority_minted: bool,
    callback_count: usize,
    trace: Vec<TraceEvent>,
    outcome: WorkerOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TraceEvent {
    IdentityAttested { identity: String },
    SandboxAttested { kind: SandboxKind },
    OutboundNetworkDenied,
    AmbientFilesystemDenied,
    InheritedFileDescriptorsClosed,
    CapabilityEnvironmentDenied,
    CapabilityDescriptorsDenied { descriptors: Vec<String> },
    CallbackInvoked { transition_ref: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WorkerOutcome {
    ExactMatched,
    ExactMismatch { transition_ref: String },
    Unavailable { reason: UnavailableReason },
}

fn worker_response(request: WorkerRequest) -> WorkerResponse {
    let identity =
        executable_identity_for_path(&std::env::current_exe().expect("worker executable path"))
            .expect("worker executable identity");
    let mut response = WorkerResponse {
        protocol: PROTOCOL.to_owned(),
        self_attested_identity: identity.clone(),
        live_authority_minted: false,
        callback_count: 0,
        trace: vec![TraceEvent::IdentityAttested {
            identity: identity.clone(),
        }],
        outcome: WorkerOutcome::Unavailable {
            reason: UnavailableReason::ProtocolViolation,
        },
    };
    if request.protocol != PROTOCOL {
        return response;
    }
    if identity != request.expected_executable_identity {
        response.outcome = WorkerOutcome::Unavailable {
            reason: UnavailableReason::ExecutableIdentityMismatch,
        };
        return response;
    }
    if let Err(reason) = attest_sandbox(&request.sandbox) {
        response.outcome = WorkerOutcome::Unavailable { reason };
        return response;
    }
    response.trace.push(TraceEvent::SandboxAttested {
        kind: request.sandbox.kind,
    });
    response.trace.push(TraceEvent::OutboundNetworkDenied);
    response.trace.push(TraceEvent::AmbientFilesystemDenied);
    response
        .trace
        .push(TraceEvent::InheritedFileDescriptorsClosed);
    if !sandbox_environment_is_capability_free(&request.sandbox) {
        response.outcome = WorkerOutcome::Unavailable {
            reason: UnavailableReason::CapabilityEnvironmentPresent,
        };
        return response;
    }
    response.trace.push(TraceEvent::CapabilityEnvironmentDenied);
    if !request.capability_descriptors.is_empty() {
        response
            .trace
            .push(TraceEvent::CapabilityDescriptorsDenied {
                descriptors: request.capability_descriptors,
            });
        response.outcome = WorkerOutcome::Unavailable {
            reason: UnavailableReason::CapabilityDescriptorDenied,
        };
        return response;
    }

    run_exact_callbacks(&mut response, request.transitions);
    response
}

fn sandbox_environment_is_capability_free(expectation: &SandboxExpectation) -> bool {
    let mut worker_marker_seen = false;
    let mut sandbox_working_directory_seen = false;
    for (name, value) in std::env::vars_os() {
        if name == WORKER_ENV && value == "1" && !worker_marker_seen {
            worker_marker_seen = true;
        } else if name == "PWD"
            && value == expectation.scratch_path.as_str()
            && !sandbox_working_directory_seen
        {
            // Bubblewrap synthesizes PWD after --clearenv; it names only the private scratch mount.
            sandbox_working_directory_seen = true;
        } else {
            return false;
        }
    }
    worker_marker_seen
}

fn attest_sandbox(expectation: &SandboxExpectation) -> Result<(), UnavailableReason> {
    let executable =
        std::env::current_exe().map_err(|_| UnavailableReason::SandboxAttestationMissing)?;
    if executable != Path::new(&expectation.expected_executable_path) {
        return Err(UnavailableReason::SandboxAttestationMissing);
    }
    if OpenOptions::new().write(true).open(&executable).is_ok() {
        return Err(UnavailableReason::SandboxAttestationMissing);
    }

    let scratch = Path::new(&expectation.scratch_path);
    let scratch_probe = scratch.join(format!("mfm-scratch-probe-{}", std::process::id()));
    fs::write(&scratch_probe, b"scratch")
        .map_err(|_| UnavailableReason::SandboxAttestationMissing)?;
    fs::remove_file(scratch_probe).map_err(|_| UnavailableReason::SandboxAttestationMissing)?;

    if fs::read(&expectation.ambient_probe_path).is_ok()
        || fs::read(Path::new("/etc/passwd")).is_ok()
    {
        return Err(UnavailableReason::SandboxAttestationMissing);
    }
    attest_outbound_network_denial()?;

    if has_unexpected_inherited_file_descriptor()? {
        return Err(UnavailableReason::InheritedFileDescriptorPresent);
    }

    #[cfg(target_os = "linux")]
    attest_linux_namespaces(expectation)?;
    #[cfg(target_os = "macos")]
    attest_macos_sandbox_kind(expectation)?;
    Ok(())
}

fn attest_outbound_network_denial() -> Result<(), UnavailableReason> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 9);
    let Err(error) = TcpStream::connect_timeout(&address, Duration::from_millis(50)) else {
        return Err(UnavailableReason::SandboxAttestationMissing);
    };

    #[cfg(target_os = "linux")]
    let denied = error.raw_os_error() == Some(101);
    #[cfg(target_os = "macos")]
    let denied = error.kind() == io::ErrorKind::PermissionDenied;

    if denied {
        Ok(())
    } else {
        Err(UnavailableReason::SandboxAttestationMissing)
    }
}

fn has_unexpected_inherited_file_descriptor() -> Result<bool, UnavailableReason> {
    #[cfg(target_os = "linux")]
    let directory = Path::new("/proc/self/fd");
    #[cfg(target_os = "macos")]
    let directory = Path::new("/dev/fd");

    let entries =
        fs::read_dir(directory).map_err(|_| UnavailableReason::SandboxAttestationMissing)?;
    for entry in entries {
        let entry = entry.map_err(|_| UnavailableReason::SandboxAttestationMissing)?;
        let Some(fd) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if fd <= 2 {
            continue;
        }
        let target = fs::read_link(entry.path())
            .map_err(|_| UnavailableReason::SandboxAttestationMissing)?;
        let target = target.to_string_lossy();
        if target.ends_with("/fd") {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

#[cfg(target_os = "linux")]
fn attest_linux_namespaces(expectation: &SandboxExpectation) -> Result<(), UnavailableReason> {
    if expectation.kind != SandboxKind::LinuxBubblewrap || expectation.parent_namespaces.len() != 6
    {
        return Err(UnavailableReason::SandboxAttestationMissing);
    }
    for parent in &expectation.parent_namespaces {
        let current = fs::read_link(Path::new("/proc/self/ns").join(&parent.name))
            .map_err(|_| UnavailableReason::SandboxAttestationMissing)?
            .to_string_lossy()
            .into_owned();
        if current == parent.identity {
            return Err(UnavailableReason::SandboxAttestationMissing);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn attest_macos_sandbox_kind(expectation: &SandboxExpectation) -> Result<(), UnavailableReason> {
    if expectation.kind != SandboxKind::MacOsSandboxExec
        || !expectation.parent_namespaces.is_empty()
    {
        return Err(UnavailableReason::SandboxAttestationMissing);
    }
    Ok(())
}

fn run_exact_callbacks(response: &mut WorkerResponse, transitions: Vec<TransitionRequest>) {
    for transition in transitions {
        response.callback_count += 1;
        response.trace.push(TraceEvent::CallbackInvoked {
            transition_ref: transition.transition_ref.clone(),
        });
        if transition.callback_output != transition.recorded_output {
            response.outcome = WorkerOutcome::ExactMismatch {
                transition_ref: transition.transition_ref,
            };
            return;
        }
    }
    response.outcome = WorkerOutcome::ExactMatched;
}

fn executable_identity_for_path(path: &Path) -> io::Result<String> {
    executable_identity_for_bytes(&fs::read(path)?)
}

fn executable_identity_for_bytes(bytes: &[u8]) -> io::Result<String> {
    let raw_sha256 = mfm_canonical::sha256_digest_bytes(bytes).to_string();
    let identity = CanonicalValue::object([
        (
            "contract",
            CanonicalValue::String(EXECUTABLE_IDENTITY_CONTRACT.to_owned()),
        ),
        ("sha256", CanonicalValue::String(raw_sha256)),
    ])
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let contract = RecoverabilityContract::embedded()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let descriptor = contract
        .encode(EXECUTABLE_DESCRIPTOR_CONTRACT, &identity)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    contract
        .content_ref(&descriptor)
        .map(|reference| reference.content_digest().to_string())
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
}

fn exact_plan_for_adapter(
    artifact: &RetainedExecutable,
    transitions: &[TransitionRef],
) -> ExactReproductionPlan {
    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    let run_id = RunId::from_str(
        "run:sha256-jcs-v1:\
         0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect("run id");
    let commit_digest = JournalCommitDigest::from_str(
        "sha256-jcs-v1:\
         1111111111111111111111111111111111111111111111111111111111111111",
    )
    .expect("commit digest");
    let semantic_head = JournalHead::new(2, &commit_digest).expect("semantic head");
    let portable_export_ref = ContentRef::new(
        contract
            .schema_id("mfm.portable-run-export-stream.v2")
            .expect("portable schema")
            .clone(),
        ContentDigest::from_str(
            "content:sha256-v1:\
             2222222222222222222222222222222222222222222222222222222222222222",
        )
        .expect("portable digest"),
    )
    .expect("portable content ref");
    let executable_identity_ref = ContentRef::new(
        contract
            .schema_id(EXECUTABLE_DESCRIPTOR_CONTRACT)
            .expect("executable descriptor schema")
            .clone(),
        ContentDigest::from_str(artifact.retained_identity()).expect("executable digest"),
    )
    .expect("executable identity ref");
    let state_manifest_ref = ContentRef::new(
        contract
            .schema_id("mfm.state-implementation-manifest.v1")
            .expect("state manifest schema")
            .clone(),
        ContentDigest::from_str(
            "content:sha256-v1:\
             3333333333333333333333333333333333333333333333333333333333333333",
        )
        .expect("state manifest digest"),
    )
    .expect("state manifest ref");
    let value = CanonicalValue::object([
        (
            "version",
            CanonicalValue::String("mfm.exact-reproduction-plan.v1".to_owned()),
        ),
        ("run_id", CanonicalValue::String(run_id.as_str().to_owned())),
        (
            "semantic_head",
            semantic_head
                .canonical_value()
                .expect("semantic head value"),
        ),
        (
            "portable_export_ref",
            content_ref_value(&portable_export_ref),
        ),
        (
            "admitted_executable_identity_ref",
            content_ref_value(&executable_identity_ref),
        ),
        (
            "state_implementation_manifest_ref",
            content_ref_value(&state_manifest_ref),
        ),
        (
            "ordered_transition_refs",
            CanonicalValue::Array(
                transitions
                    .iter()
                    .map(|reference| reference.canonical_value().expect("transition ref"))
                    .collect(),
            ),
        ),
    ])
    .expect("exact plan value");
    let encoded = contract
        .encode("mfm.exact-reproduction-plan.v1", &value)
        .expect("exact plan");
    ExactReproductionPlan::strict_decode(encoded.as_bytes()).expect("strict exact plan")
}

fn content_ref_value(reference: &ContentRef) -> CanonicalValue {
    CanonicalValue::object([
        (
            "schema_id",
            CanonicalValue::String(reference.schema_id().as_str().to_owned()),
        ),
        (
            "content_digest",
            CanonicalValue::String(reference.content_digest().as_str().to_owned()),
        ),
    ])
    .expect("content ref value")
}

fn adapter_transition_ref() -> TransitionRef {
    let run_id = RunId::from_str(
        "run:sha256-jcs-v1:\
         0000000000000000000000000000000000000000000000000000000000000000",
    )
    .expect("run id");
    let record_hash = JournalRecordHash::from_str(
        "sha256-jcs-v1:\
         4444444444444444444444444444444444444444444444444444444444444444",
    )
    .expect("record hash");
    let record_ref = RecordRef::new(&run_id, 2, 0, &record_hash).expect("record ref");
    TransitionRef::new(&record_ref).expect("transition ref")
}

#[test]
#[ignore = "launched only as the retained capability-free executable"]
fn retained_executable_worker() {
    if std::env::var_os(WORKER_ENV).is_none() {
        return;
    }
    let mut request = Vec::new();
    io::stdin()
        .read_to_end(&mut request)
        .expect("read capability-free protocol request");
    let request: WorkerRequest =
        serde_json::from_slice(&request).expect("parse capability-free protocol request");
    let response = worker_response(request);
    println!(
        "{RESPONSE_PREFIX}{}",
        serde_json::to_string(&response).expect("serialize capability-free protocol response")
    );
}

#[test]
fn structural_mode_invokes_no_executable_or_callback() {
    let launcher = PrototypeLauncher::new();
    let history = vec![
        TransitionRequest::comparable("transition:1", "one", "different callback result"),
        TransitionRequest::comparable("transition:2", "two", "two"),
    ];

    let verified = verify_recorded_history_structurally(&history).expect("structural history");

    assert_eq!(verified.transition_count, 2);
    assert_eq!(
        launcher.launch_count(),
        0,
        "callback code is reachable only in the child executable"
    );
}

#[test]
fn replay_resolver_receives_only_strict_canonical_plan_bytes() {
    let artifact = RetainedExecutable::capture().expect("retained executable");
    let launcher = PrototypeLauncher::new();
    let resolver = QualifiedResolverAdapter {
        launcher: &launcher,
        artifact: &artifact,
    };
    let transition_ref = adapter_transition_ref();
    let plan = exact_plan_for_adapter(&artifact, std::slice::from_ref(&transition_ref));

    let result = resolve_ready(resolver.reproduce_exact(plan.as_bytes()));

    #[cfg(target_os = "linux")]
    assert_eq!(result, ReplayExactReproduction::Matched);
    #[cfg(target_os = "macos")]
    assert!(matches!(
        result,
        ReplayExactReproduction::Matched | ReplayExactReproduction::Unavailable
    ));

    let unavailable_launcher = PrototypeLauncher::unavailable();
    let unavailable_resolver = QualifiedResolverAdapter {
        launcher: &unavailable_launcher,
        artifact: &artifact,
    };
    let unavailable = resolve_ready(unavailable_resolver.reproduce_exact(plan.as_bytes()));
    assert_eq!(unavailable, ReplayExactReproduction::Unavailable);
    assert_eq!(unavailable_launcher.launch_count(), 0);

    let invalid_launcher = PrototypeLauncher::new();
    let invalid_resolver = QualifiedResolverAdapter {
        launcher: &invalid_launcher,
        artifact: &artifact,
    };
    let invalid = resolve_ready(invalid_resolver.reproduce_exact(b"not a canonical plan"));
    assert_eq!(invalid, ReplayExactReproduction::Unavailable);
    assert_eq!(invalid_launcher.launch_count(), 0);
}

#[test]
fn env_clear_without_platform_sandbox_is_rejected_before_callbacks() {
    let artifact = RetainedExecutable::capture().expect("retained executable");
    let request = WorkerRequest {
        protocol: PROTOCOL.to_owned(),
        expected_executable_identity: artifact.retained_identity().to_owned(),
        sandbox: sandbox_expectation(&artifact).expect("sandbox expectation"),
        capability_descriptors: Vec::new(),
        transitions: vec![TransitionRequest::comparable(
            "transition:control",
            "recorded",
            "recorded",
        )],
    };
    let response = PrototypeLauncher::new()
        .launch_env_clear_control(artifact.path(), &request)
        .expect("rejected control response");

    assert_eq!(response.callback_count, 0);
    assert!(!response.live_authority_minted);
    assert!(matches!(
        response.outcome,
        WorkerOutcome::Unavailable {
            reason: UnavailableReason::SandboxAttestationMissing,
        }
    ));
    assert!(matches!(
        response.trace.as_slice(),
        [TraceEvent::IdentityAttested { identity }]
            if identity == artifact.retained_identity()
    ));
}

#[test]
fn unavailable_platform_sandbox_does_not_launch_the_executable() {
    let artifact = RetainedExecutable::capture().expect("retained executable");
    let launcher = PrototypeLauncher::unavailable();

    let result = launcher.reproduce_exact(
        &artifact,
        artifact.retained_identity(),
        Vec::new(),
        vec![TransitionRequest::comparable(
            "transition:must-not-run",
            "recorded",
            "recorded",
        )],
    );

    assert!(matches!(
        result,
        ExactReproduction::Unavailable {
            reason: UnavailableReason::SandboxUnavailable,
            evidence: None,
        }
    ));
    assert_eq!(launcher.launch_count(), 0);
}

#[test]
fn exact_mode_attests_identity_and_denies_environment_before_callbacks() {
    let artifact = RetainedExecutable::capture().expect("retained executable");
    assert!(artifact.directory_exists());
    let launcher = PrototypeLauncher::new();

    let result = launcher.reproduce_exact(
        &artifact,
        artifact.retained_identity(),
        Vec::new(),
        vec![TransitionRequest::comparable(
            "transition:exact",
            "recorded-output",
            "recorded-output",
        )],
    );

    #[cfg(target_os = "linux")]
    let evidence = match result {
        ExactReproduction::Matched(evidence) => evidence,
        _ => panic!("Linux Bubblewrap qualification did not match: {result:?}"),
    };
    #[cfg(target_os = "macos")]
    let evidence = match result {
        ExactReproduction::Matched(evidence) => evidence,
        ExactReproduction::Unavailable {
            reason: UnavailableReason::SandboxUnavailable,
            evidence: None,
        } => return,
        _ => panic!("macOS sandbox qualification did not match: {result:?}"),
    };
    assert_eq!(launcher.launch_count(), 1);
    assert_eq!(
        evidence.self_attested_identity,
        artifact.retained_identity()
    );
    assert_eq!(evidence.callback_count, 1);
    assert!(!evidence.live_authority_minted);
    assert!(matches!(
        evidence.trace.as_slice(),
        [
            TraceEvent::IdentityAttested { identity },
            TraceEvent::SandboxAttested { kind },
            TraceEvent::OutboundNetworkDenied,
            TraceEvent::AmbientFilesystemDenied,
            TraceEvent::InheritedFileDescriptorsClosed,
            TraceEvent::CapabilityEnvironmentDenied,
            TraceEvent::CallbackInvoked { transition_ref },
        ] if identity == artifact.retained_identity()
            && *kind == platform_sandbox_kind()
            && transition_ref == "transition:exact"
    ));
}

#[test]
fn identity_and_capability_descriptor_mismatches_stop_before_callbacks() {
    let artifact = RetainedExecutable::capture().expect("retained executable");
    let launcher = PrototypeLauncher::new();
    let other_identity =
        executable_identity_for_bytes(b"different admitted executable").expect("other identity");

    let wrong_identity = launcher.reproduce_exact(
        &artifact,
        &other_identity,
        Vec::new(),
        vec![TransitionRequest::comparable(
            "transition:identity",
            "recorded",
            "recorded",
        )],
    );
    if matches!(
        wrong_identity,
        ExactReproduction::Unavailable {
            reason: UnavailableReason::SandboxUnavailable,
            evidence: None,
        }
    ) {
        #[cfg(target_os = "macos")]
        {
            return;
        }
        #[cfg(target_os = "linux")]
        panic!("Linux Bubblewrap became unavailable during identity conformance");
    }
    let ExactReproduction::Unavailable {
        reason: UnavailableReason::ExecutableIdentityMismatch,
        evidence: Some(identity_evidence),
    } = wrong_identity
    else {
        panic!("identity mismatch was not unavailable: {wrong_identity:?}");
    };
    assert_eq!(identity_evidence.callback_count, 0);
    assert!(matches!(
        identity_evidence.trace.as_slice(),
        [TraceEvent::IdentityAttested { identity }]
            if identity == artifact.retained_identity()
    ));

    let denied_descriptor = "mfm.capability.network.v1".to_owned();
    let denied = launcher.reproduce_exact(
        &artifact,
        artifact.retained_identity(),
        vec![denied_descriptor.clone()],
        vec![TransitionRequest::comparable(
            "transition:capability",
            "recorded",
            "recorded",
        )],
    );
    let ExactReproduction::Unavailable {
        reason: UnavailableReason::CapabilityDescriptorDenied,
        evidence: Some(denial_evidence),
    } = denied
    else {
        panic!("capability descriptor was not denied: {denied:?}");
    };
    assert_eq!(denial_evidence.callback_count, 0);
    assert!(matches!(
        denial_evidence.trace.as_slice(),
        [
            TraceEvent::IdentityAttested { .. },
            TraceEvent::SandboxAttested { .. },
            TraceEvent::OutboundNetworkDenied,
            TraceEvent::AmbientFilesystemDenied,
            TraceEvent::InheritedFileDescriptorsClosed,
            TraceEvent::CapabilityEnvironmentDenied,
            TraceEvent::CapabilityDescriptorsDenied { descriptors },
        ] if descriptors == &[denied_descriptor]
    ));
}

#[test]
fn missing_or_changed_retained_artifact_is_unavailable_without_launch() {
    let missing = RetainedExecutable::capture().expect("missing executable fixture");
    missing.remove().expect("remove retained executable");
    let missing_launcher = PrototypeLauncher::new();
    let missing_result = missing_launcher.reproduce_exact(
        &missing,
        missing.retained_identity(),
        Vec::new(),
        Vec::new(),
    );
    assert!(matches!(
        missing_result,
        ExactReproduction::Unavailable {
            reason: UnavailableReason::ArtifactMissing,
            evidence: None,
        }
    ));
    assert_eq!(missing_launcher.launch_count(), 0);

    let changed = RetainedExecutable::capture().expect("changed executable fixture");
    changed
        .append_unadmitted_bytes()
        .expect("change retained executable");
    let changed_launcher = PrototypeLauncher::new();
    let changed_result = changed_launcher.reproduce_exact(
        &changed,
        changed.retained_identity(),
        Vec::new(),
        Vec::new(),
    );
    assert!(matches!(
        changed_result,
        ExactReproduction::Unavailable {
            reason: UnavailableReason::ArtifactChanged,
            evidence: None,
        }
    ));
    assert_eq!(changed_launcher.launch_count(), 0);
}
