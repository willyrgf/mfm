pub mod allowance;
pub mod approve;
pub mod asset;
pub mod balances;
pub mod cmd;
pub mod config;
pub mod encrypt;
pub mod quote;
pub mod rebalancer;
pub mod swap;
pub mod telemetry;
pub mod track;
pub mod unwrap;
pub mod utils;
pub mod withdraw;
pub mod wrap;

pub const APP_NAME: &str = "mfm";
pub const DEFAULT_LOG_LEVEL: &str = "warn";

// Since the exit code names e.g. `SIGBUS` are most appropriate yet trigger a test error with the
// clippy lint `upper_case_acronyms` we have disabled this lint for this enum.
/// Vmm exit-code type.
/// copied from https://github.com/firecracker-microvm/firecracker/blob/9b165839f7c3593e165ce35ae13b3c48f7bb661e/src/vmm/src/lib.rs#L73
/// TODO: refactor it based on our needs
#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    /// Success exit code.
    Ok = 0,
    /// Generic error exit code.
    GenericError = 1,
    /// Generic exit code for an error considered not possible to occur if the program logic is
    /// sound.
    UnexpectedError = 2,
    /// Firecracker was shut down after intercepting a restricted system call.
    BadSyscall = 148,
    /// Firecracker was shut down after intercepting `SIGBUS`.
    SIGBUS = 149,
    /// Firecracker was shut down after intercepting `SIGSEGV`.
    SIGSEGV = 150,
    /// Firecracker was shut down after intercepting `SIGXFSZ`.
    SIGXFSZ = 151,
    /// Firecracker was shut down after intercepting `SIGXCPU`.
    SIGXCPU = 154,
    /// Firecracker was shut down after intercepting `SIGPIPE`.
    SIGPIPE = 155,
    /// Firecracker was shut down after intercepting `SIGHUP`.
    SIGHUP = 156,
    /// Firecracker was shut down after intercepting `SIGILL`.
    SIGILL = 157,
    /// Bad configuration for microvm's resources, when using a single json.
    BadConfiguration = 152,
    /// Command line arguments parsing error.
    ArgParsing = 153,
}
