use std::fmt;
use std::path::{Path, PathBuf};

/// Source kind used to resolve a runtime-local secret value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeValueSourceKind {
    /// Value was supplied directly in the runtime config.
    Direct,
    /// Value was read from an environment variable.
    Env,
    /// Value was read from a file path in the runtime config.
    File,
    /// Value was read from a file path obtained from an environment variable.
    FileEnv,
}

/// Resolved runtime-local value with redacted debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeSecretValue {
    value: String,
    source_kind: RuntimeValueSourceKind,
}

impl RuntimeSecretValue {
    /// Returns the resolved value.
    ///
    /// Callers must keep this below persisted, public, and replay surfaces.
    pub fn expose_secret(&self) -> &str {
        &self.value
    }

    /// Returns the source kind used to resolve this value.
    pub const fn source_kind(&self) -> RuntimeValueSourceKind {
        self.source_kind
    }

    pub(super) fn from_parts(value: String, source_kind: RuntimeValueSourceKind) -> Self {
        Self { value, source_kind }
    }
}

impl fmt::Debug for RuntimeSecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSecretValue")
            .field("value", &"<redacted>")
            .field("source_kind", &self.source_kind)
            .finish()
    }
}

/// Resolved runtime-local path with redacted debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeSecretPath {
    path: PathBuf,
    source_kind: RuntimeValueSourceKind,
}

impl RuntimeSecretPath {
    /// Returns the resolved path.
    ///
    /// Callers must keep this below persisted, public, and replay surfaces.
    pub fn expose_path(&self) -> &Path {
        &self.path
    }

    /// Returns the source kind used to resolve this path.
    pub const fn source_kind(&self) -> RuntimeValueSourceKind {
        self.source_kind
    }

    pub(super) fn from_value(value: RuntimeSecretValue) -> Self {
        Self {
            path: PathBuf::from(value.value),
            source_kind: value.source_kind,
        }
    }
}

impl fmt::Debug for RuntimeSecretPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSecretPath")
            .field("path", &"<redacted>")
            .field("source_kind", &self.source_kind)
            .finish()
    }
}
