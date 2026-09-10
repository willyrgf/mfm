//! Closed reviewed diagnostic fields.

use super::*;

/// Closed chain-end vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "chain-end",
    version = "1",
    schema = "mfm.diagnostics.chain-end"
)]
pub enum ChainEnd {
    /// All exposed sources were retained.
    Complete,
    /// The upstream API did not expose the evidence.
    Unavailable,
    /// Capture stopped at its finite bound.
    BoundReached,
}

/// Closed source-kind vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "source-kind",
    version = "1",
    schema = "mfm.diagnostics.source-kind"
)]
pub enum SourceKind {
    /// Transport source category.
    Transport,
    /// Database source category.
    Database,
    /// Os source category.
    Os,
    /// Parse source category.
    Parse,
    /// Task source category.
    Task,
    /// Channel source category.
    Channel,
    /// Crypto source category.
    Crypto,
    /// Opaque source category.
    Opaque,
}

/// Closed omitted-field vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "omitted-field",
    version = "1",
    schema = "mfm.diagnostics.omitted-field"
)]
pub enum OmittedField {
    /// Unretained message.
    Message,
    /// Unretained data.
    Data,
    /// Unretained body.
    Body,
    /// Unretained url.
    Url,
    /// Unretained databasedetail.
    DatabaseDetail,
    /// Unretained parameters.
    Parameters,
    /// Unretained panicpayload.
    PanicPayload,
    /// Unretained sourcedetail.
    SourceDetail,
}

/// Closed omission-reason vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "omission-reason",
    version = "1",
    schema = "mfm.diagnostics.omission-reason"
)]
pub enum OmissionReason {
    /// The disclosure contract excludes the exposed field.
    Withheld,
    /// The upstream API did not expose the field.
    Unavailable,
    /// The capture budget excluded the field.
    BoundReached,
}

/// Closed transport-failure-kind vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "transport-failure-kind",
    version = "1",
    schema = "mfm.diagnostics.transport-failure-kind"
)]
pub enum TransportFailureKind {
    /// Timeout transport failure.
    Timeout,
    /// Connect transport failure.
    Connect,
    /// Request transport failure.
    Request,
    /// Body transport failure.
    Body,
    /// Decode transport failure.
    Decode,
    /// Redirect transport failure.
    Redirect,
}

/// Closed database-failure-kind vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "database-failure-kind",
    version = "1",
    schema = "mfm.diagnostics.database-failure-kind"
)]
pub enum DatabaseFailureKind {
    /// Connect database failure.
    Connect,
    /// Database database failure.
    Database,
    /// Io database failure.
    Io,
    /// Tls database failure.
    Tls,
    /// Protocol database failure.
    Protocol,
    /// Decode database failure.
    Decode,
    /// ColumnNotFound database failure.
    ColumnNotFound,
    /// RowNotFound database failure.
    RowNotFound,
    /// PoolTimedOut database failure.
    PoolTimedOut,
    /// PoolClosed database failure.
    PoolClosed,
    /// WorkerCrashed database failure.
    WorkerCrashed,
    /// Other database failure.
    Other,
}

/// Closed os-failure-kind vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "os-failure-kind",
    version = "1",
    schema = "mfm.diagnostics.os-failure-kind"
)]
pub enum OsFailureKind {
    /// NotFound operating-system failure.
    NotFound,
    /// PermissionDenied operating-system failure.
    PermissionDenied,
    /// ConnectionRefused operating-system failure.
    ConnectionRefused,
    /// ConnectionReset operating-system failure.
    ConnectionReset,
    /// ConnectionAborted operating-system failure.
    ConnectionAborted,
    /// NotConnected operating-system failure.
    NotConnected,
    /// AddrInUse operating-system failure.
    AddrInUse,
    /// AddrNotAvailable operating-system failure.
    AddrNotAvailable,
    /// BrokenPipe operating-system failure.
    BrokenPipe,
    /// AlreadyExists operating-system failure.
    AlreadyExists,
    /// WouldBlock operating-system failure.
    WouldBlock,
    /// InvalidInput operating-system failure.
    InvalidInput,
    /// InvalidData operating-system failure.
    InvalidData,
    /// TimedOut operating-system failure.
    TimedOut,
    /// WriteZero operating-system failure.
    WriteZero,
    /// Interrupted operating-system failure.
    Interrupted,
    /// UnexpectedEof operating-system failure.
    UnexpectedEof,
    /// OutOfMemory operating-system failure.
    OutOfMemory,
    /// Other operating-system failure.
    Other,
}

/// Closed parse-category vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "parse-category",
    version = "1",
    schema = "mfm.diagnostics.parse-category"
)]
pub enum ParseCategory {
    /// Io parser failure.
    Io,
    /// Syntax parser failure.
    Syntax,
    /// Data parser failure.
    Data,
    /// Eof parser failure.
    Eof,
}

/// Closed task-failure-kind vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "task-failure-kind",
    version = "1",
    schema = "mfm.diagnostics.task-failure-kind"
)]
pub enum TaskFailureKind {
    /// The task was cancelled.
    Cancelled,
    /// The task panicked; payload is excluded.
    Panicked,
}

/// Closed channel-failure-kind vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "channel-failure-kind",
    version = "1",
    schema = "mfm.diagnostics.channel-failure-kind"
)]
pub enum ChannelFailureKind {
    /// The channel closed.
    Closed,
    /// The receiving owner is unavailable.
    OwnerUnavailable,
}

/// Location exposed by the parser, without input bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "parse-location",
    version = "1",
    schema = "mfm.diagnostics.parse-location"
)]
pub enum ParseLocation {
    /// Line and column as reported by the parser.
    LineColumn {
        /// Reported line.
        line: u64,
        /// Reported column.
        column: u64,
    },
    /// Byte offset as reported by the parser.
    Offset {
        /// Reported byte offset.
        offset: u64,
    },
    /// No location was exposed.
    Unavailable,
}

/// Whether an observed size is exact or only a lower bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "observed-size",
    version = "1",
    schema = "mfm.diagnostics.observed-size"
)]
pub enum ObservedSize {
    /// Exact measured size.
    Exact {
        /// Measured bytes or items.
        value: u64,
    },
    /// Reading stopped after this many bytes or items.
    AtLeast {
        /// Known lower bound.
        value: u64,
    },
}

/// Owner of an omitted diagnostic field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "evidence-location",
    version = "1",
    schema = "mfm.diagnostics.evidence-location"
)]
pub enum EvidenceLocation {
    /// The received response.
    Response,
    /// One concrete captured source.
    SourceLayer {
        /// Zero-based source index.
        index: u8,
    },
}

/// Reviewed structured facts about one concrete source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, PersistedSchema)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.diagnostics",
    name = "source-fact",
    version = "1",
    schema = "mfm.diagnostics.source-fact"
)]
pub enum SourceFact {
    /// Transport evidence.
    Transport {
        /// Reviewed kind.
        kind: TransportFailureKind,
    },
    /// Database evidence.
    Database {
        /// Reviewed kind.
        kind: DatabaseFailureKind,
    },
    /// SqlState evidence.
    SqlState {
        /// Reviewed code.
        code: SqlState,
    },
    /// Os evidence.
    Os {
        /// Reviewed kind.
        kind: OsFailureKind,
    },
    /// OsCode evidence.
    OsCode {
        /// Reviewed code.
        code: i32,
    },
    /// Parse evidence.
    Parse {
        /// Reviewed category.
        category: ParseCategory,
        /// Exposed parser location.
        location: ParseLocation,
    },
    /// Size evidence.
    Size {
        /// Reviewed limit.
        limit: u64,
        /// Measured size.
        observed: ObservedSize,
    },
    /// Task evidence.
    Task {
        /// Reviewed outcome.
        outcome: TaskFailureKind,
    },
    /// Channel evidence.
    Channel {
        /// Reviewed outcome.
        outcome: ChannelFailureKind,
    },
}
