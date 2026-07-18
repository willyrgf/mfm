use super::RuntimeConfigFormat;
use std::fmt;

/// Redaction-safe runtime config error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("runtime config {location}: {kind}")]
pub struct RuntimeConfigError {
    location: RuntimeConfigLocation,
    kind: RuntimeConfigErrorKind,
}

impl RuntimeConfigError {
    pub(crate) fn new(location: RuntimeConfigLocation, kind: RuntimeConfigErrorKind) -> Self {
        Self { location, kind }
    }

    /// Returns the redaction-safe error location.
    pub const fn location(&self) -> &RuntimeConfigLocation {
        &self.location
    }

    /// Returns the closed error kind.
    pub const fn kind(&self) -> &RuntimeConfigErrorKind {
        &self.kind
    }
}

/// Redaction-safe runtime config error location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeConfigLocation {
    /// Top-level runtime config.
    Root,
    /// EVM capability family.
    Evm,
    /// Bitcoin capability family.
    Btc,
    /// Bitcoin semantic source route entry.
    BtcRoute {
        /// Checked semantic source identity when available.
        source_identity: Option<String>,
    },
    /// EVM semantic network route entry.
    EvmRoute {
        /// Checked semantic network id when available.
        network_id: Option<String>,
    },
    /// Keystore profile entry.
    Keystore {
        /// Checked keystore profile ref when available.
        keystore_ref: Option<String>,
    },
    /// Runtime signer entry.
    Signer {
        /// Checked signer ref when available.
        signer_ref: Option<String>,
    },
    /// Known field within a parent location.
    Field {
        /// Parent location.
        parent: Box<RuntimeConfigLocation>,
        /// Redaction-safe field name.
        field: &'static str,
    },
}

impl RuntimeConfigLocation {
    pub(crate) fn with_field(self, field: &'static str) -> Self {
        Self::Field {
            parent: Box::new(self),
            field,
        }
    }
}

impl fmt::Display for RuntimeConfigLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("root"),
            Self::Evm => f.write_str("evm"),
            Self::Btc => f.write_str("btc"),
            Self::BtcRoute { source_identity } => match source_identity {
                Some(source_identity) => write!(f, "btc.routes[{source_identity}]"),
                None => f.write_str("btc.routes[<invalid>]"),
            },
            Self::EvmRoute { network_id } => match network_id {
                Some(network_id) => write!(f, "evm.routes[{network_id}]"),
                None => f.write_str("evm.routes[<invalid>]"),
            },
            Self::Keystore { keystore_ref } => match keystore_ref {
                Some(keystore_ref) => write!(f, "keystores[{keystore_ref}]"),
                None => f.write_str("keystores[<invalid>]"),
            },
            Self::Signer { signer_ref } => match signer_ref {
                Some(signer_ref) => write!(f, "signers[{signer_ref}]"),
                None => f.write_str("signers[<invalid>]"),
            },
            Self::Field { parent, field } => write!(f, "{parent}.{field}"),
        }
    }
}

/// Closed runtime config error kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeConfigErrorKind {
    /// File extension does not identify TOML or JSON.
    UnsupportedFormat,
    /// Runtime config syntax could not be parsed.
    Syntax {
        /// Runtime config format being parsed.
        format: RuntimeConfigFormat,
    },
    /// Runtime config file could not be read.
    ConfigFileRead,
    /// A required capability family was missing.
    MissingFamily,
    /// Capability family section did not have the expected shape.
    InvalidFamilyConfig,
    /// Runtime config contained an unknown field.
    UnknownField,
    /// Runtime config contained forbidden source chain metadata.
    ForbiddenExpectedChainId,
    /// Runtime config contained forbidden secret-bearing material.
    ForbiddenSecretMaterial,
    /// Runtime config identifier failed its grammar.
    InvalidIdentifier {
        /// Identifier kind.
        kind: RuntimeConfigIdentifierKind,
    },
    /// Required field was not present.
    MissingRequiredField,
    /// Required value source did not provide exactly one source.
    ExactlyOneValueSource,
    /// Optional value source provided more than one source.
    AtMostOneValueSource,
    /// Runtime environment variable was missing.
    MissingEnv,
    /// Runtime environment variable was not usable.
    InvalidEnv,
    /// Indirection file could not be read.
    IndirectionFileRead,
    /// Resolved value was empty.
    EmptyResolvedValue,
    /// RPC URL was not syntactically valid.
    InvalidUrl,
    /// RPC URL contained userinfo.
    UrlUserInfo,
    /// Requested semantic route was missing.
    MissingRoute,
    /// Bitcoin JSON-RPC basic authentication had only one of user/password.
    IncompleteBasicAuth,
    /// Signer provider was unsupported.
    UnsupportedSignerProvider,
    /// Signer binding section did not have the expected shape.
    InvalidSignerConfig,
    /// Keystore profile section did not have the expected shape.
    InvalidKeystoreConfig,
    /// Referenced keystore profile was missing.
    MissingKeystore,
    /// Requested signer binding was missing.
    MissingSigner,
    /// Keystore entry id was malformed.
    InvalidEntryId,
}

impl fmt::Display for RuntimeConfigErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => f.write_str("unsupported format"),
            Self::Syntax { format } => write!(f, "{format:?} syntax is invalid"),
            Self::ConfigFileRead => f.write_str("config file could not be read"),
            Self::MissingFamily => f.write_str("required capability family is missing"),
            Self::InvalidFamilyConfig => f.write_str("capability family config is invalid"),
            Self::UnknownField => f.write_str("unknown field"),
            Self::ForbiddenExpectedChainId => {
                f.write_str("source-level expected_chain_id is forbidden")
            }
            Self::ForbiddenSecretMaterial => f.write_str("forbidden secret material field"),
            Self::InvalidIdentifier { kind } => write!(f, "invalid {kind}"),
            Self::MissingRequiredField => f.write_str("required field is missing"),
            Self::ExactlyOneValueSource => {
                f.write_str("exactly one value source must be configured")
            }
            Self::AtMostOneValueSource => f.write_str("at most one value source may be configured"),
            Self::MissingEnv => f.write_str("runtime environment variable is missing"),
            Self::InvalidEnv => f.write_str("runtime environment variable is invalid"),
            Self::IndirectionFileRead => f.write_str("indirection file could not be read"),
            Self::EmptyResolvedValue => f.write_str("resolved value is empty"),
            Self::InvalidUrl => f.write_str("RPC URL is invalid"),
            Self::UrlUserInfo => f.write_str("RPC URL userinfo is forbidden"),
            Self::MissingRoute => f.write_str("requested EVM route is missing"),
            Self::IncompleteBasicAuth => {
                f.write_str("basic authentication requires both user and password")
            }
            Self::UnsupportedSignerProvider => f.write_str("signer provider is unsupported"),
            Self::InvalidSignerConfig => f.write_str("signer config is invalid"),
            Self::InvalidKeystoreConfig => f.write_str("keystore config is invalid"),
            Self::MissingKeystore => f.write_str("referenced keystore profile is missing"),
            Self::MissingSigner => f.write_str("requested signer binding is missing"),
            Self::InvalidEntryId => f.write_str("keystore entry id is invalid"),
        }
    }
}

/// Closed runtime config identifier kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigIdentifierKind {
    /// EVM source reference.
    SourceRef,
    /// Bitcoin semantic source identity.
    BtcSourceIdentity,
    /// EVM semantic network id.
    NetworkId,
    /// Signer reference.
    SignerRef,
    /// Keystore profile reference.
    KeystoreRef,
    /// Runtime environment variable name.
    EnvName,
}

impl fmt::Display for RuntimeConfigIdentifierKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceRef => f.write_str("source_ref"),
            Self::BtcSourceIdentity => f.write_str("btc_source_identity"),
            Self::NetworkId => f.write_str("network_id"),
            Self::SignerRef => f.write_str("signer_ref"),
            Self::KeystoreRef => f.write_str("keystore_ref"),
            Self::EnvName => f.write_str("env_name"),
        }
    }
}
