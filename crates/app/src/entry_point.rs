use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use mfm_authored_config::{AuthoredConfig, AuthoredConfigError, EntryPointDescriptor};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, NameToken, ResourceNamespace};
use mfm_program::TypedProgramLaunchPlan;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Public entry-point operation name accepted by CLI and REST transports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicOpName(NameToken);

impl PublicOpName {
    /// Creates a checked public operation name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, EntryPointOpError> {
        let value = value.as_ref();
        NameToken::new(value).map(Self).map_err(|error| {
            EntryPointOpError::new(
                "InvalidPublicOpName",
                format!("public op name is invalid: {error}"),
            )
        })
    }

    /// Returns the public name as transport text.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PublicOpName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PublicOpName {
    type Err = EntryPointOpError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Integer public version selector for an entry-point operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpVersion(u32);

impl OpVersion {
    /// Creates a checked non-zero public operation version.
    pub fn new(value: u32) -> Result<Self, EntryPointOpError> {
        if value == 0 {
            return Err(EntryPointOpError::new(
                "InvalidOpVersion",
                "entry-point op version must be greater than zero",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the numeric public operation version.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for OpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for OpVersion {
    type Err = EntryPointOpError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed = value.parse::<u32>().map_err(|_| {
            EntryPointOpError::new(
                "InvalidOpVersion",
                "entry-point op version must be an unsigned integer",
            )
        })?;
        Self::new(parsed)
    }
}

/// Stable typed identity for a launchable entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntryPointOpId {
    /// Domain namespace for the entry-point operation.
    pub namespace: ResourceNamespace,
    /// Stable domain operation name within the namespace.
    pub name: NameToken,
    /// Public operation version.
    pub version: OpVersion,
}

impl EntryPointOpId {
    /// Creates a checked entry-point operation id.
    pub fn new(
        namespace: impl AsRef<str>,
        name: impl AsRef<str>,
        version: OpVersion,
    ) -> Result<Self, EntryPointOpError> {
        let namespace = namespace.as_ref();
        let name = name.as_ref();
        let namespace = ResourceNamespace::new(namespace).map_err(|error| {
            EntryPointOpError::new(
                "InvalidEntryPointOpId",
                format!("entry-point op namespace is invalid: {error}"),
            )
        })?;
        let name = NameToken::new(name).map_err(|error| {
            EntryPointOpError::new(
                "InvalidEntryPointOpId",
                format!("entry-point op name is invalid: {error}"),
            )
        })?;
        Ok(Self {
            namespace,
            name,
            version,
        })
    }
}

impl fmt::Display for EntryPointOpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.namespace.as_str(),
            self.name.as_str(),
            self.version
        )
    }
}

/// Operation that can plan a public entry-point run from authored config.
pub trait LaunchableOp: Send + Sync {
    /// Returns the static public entry-point metadata for this operation.
    fn descriptor(&self) -> EntryPointDescriptor;

    /// Returns the stable entry-point operation id.
    fn op_id(&self) -> EntryPointOpId;

    /// Deterministically plans the typed program draft and launch material.
    fn plan(
        &self,
        authored_config: AuthoredConfig,
    ) -> Result<TypedProgramLaunchPlan, EntryPointOpError>;
}

/// Generic app adapter from a typed op-crate planner to [`LaunchableOp`].
pub struct EntryPointPlannerAdapter<TConfig, E> {
    descriptor: EntryPointDescriptor,
    op_id: EntryPointOpId,
    planner: fn(TConfig) -> Result<TypedProgramLaunchPlan, E>,
    map_error: fn(E) -> EntryPointOpError,
    _config: PhantomData<fn() -> TConfig>,
}

impl<TConfig, E> EntryPointPlannerAdapter<TConfig, E> {
    /// Builds a launchable adapter from app-neutral descriptor and planner exports.
    pub fn new(
        descriptor: EntryPointDescriptor,
        planner: fn(TConfig) -> Result<TypedProgramLaunchPlan, E>,
        map_error: fn(E) -> EntryPointOpError,
    ) -> Result<Self, EntryPointOpError> {
        if descriptor.accepted_config_formats.is_empty() {
            return Err(EntryPointOpError::new(
                "EntryPointOpConfigFormatsEmpty",
                "entry-point op must accept at least one config format",
            ));
        }
        let version = OpVersion::new(descriptor.version)?;
        Ok(Self {
            descriptor,
            op_id: EntryPointOpId::new(descriptor.namespace, descriptor.name, version)?,
            planner,
            map_error,
            _config: PhantomData,
        })
    }
}

impl<TConfig, E> LaunchableOp for EntryPointPlannerAdapter<TConfig, E>
where
    TConfig: DeserializeOwned + Serialize + Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    fn descriptor(&self) -> EntryPointDescriptor {
        self.descriptor
    }

    fn op_id(&self) -> EntryPointOpId {
        self.op_id.clone()
    }

    fn plan(
        &self,
        authored_config: AuthoredConfig,
    ) -> Result<TypedProgramLaunchPlan, EntryPointOpError> {
        if !self
            .descriptor
            .accepted_config_formats
            .contains(&authored_config.format())
        {
            return Err(EntryPointOpError::new(
                "EntryPointOpConfigFormatUnsupported",
                "entry-point op does not accept the supplied config format",
            ));
        }

        let normalized = authored_config.normalize::<TConfig>()?;
        let planned = (self.planner)(normalized.value).map_err(self.map_error)?;
        Ok(planned)
    }
}

/// Registry that resolves public operation names and versions to launchable ops.
#[derive(Clone, Default)]
pub struct EntryPointOpRegistry {
    ops: BTreeMap<PublicOpName, BTreeMap<OpVersion, Arc<dyn LaunchableOp>>>,
}

impl EntryPointOpRegistry {
    /// Creates an empty entry-point operation registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a launchable operation.
    pub fn register(&mut self, op: impl LaunchableOp + 'static) -> Result<(), EntryPointOpError> {
        self.register_arc(Arc::new(op))
    }

    /// Registers an already shared launchable operation.
    pub fn register_arc(&mut self, op: Arc<dyn LaunchableOp>) -> Result<(), EntryPointOpError> {
        let descriptor = op.descriptor();
        if descriptor.accepted_config_formats.is_empty() {
            return Err(EntryPointOpError::new(
                "EntryPointOpConfigFormatsEmpty",
                "entry-point op must accept at least one config format",
            ));
        }
        let public_name = PublicOpName::new(descriptor.public_name)?;
        let version = OpVersion::new(descriptor.version)?;
        let op_id = op.op_id();
        if op_id.version != version {
            return Err(EntryPointOpError::new(
                "EntryPointOpVersionMismatch",
                "entry-point op id version must match registered version",
            ));
        }

        let versions = self.ops.entry(public_name).or_default();
        if versions.contains_key(&version) {
            return Err(EntryPointOpError::new(
                "DuplicateEntryPointOp",
                "entry-point op public name and version are already registered",
            ));
        }
        versions.insert(version, op);
        Ok(())
    }

    /// Resolves a public name to the latest registered version.
    pub fn resolve_latest(
        &self,
        public_name: &PublicOpName,
    ) -> Result<Arc<dyn LaunchableOp>, EntryPointOpError> {
        let versions = self.ops.get(public_name).ok_or_else(|| {
            EntryPointOpError::new("EntryPointOpNotFound", "entry-point op is not registered")
        })?;
        versions
            .last_key_value()
            .map(|(_version, op)| Arc::clone(op))
            .ok_or_else(|| {
                EntryPointOpError::new("EntryPointOpNotFound", "entry-point op is not registered")
            })
    }

    /// Resolves a public name and optional explicit version.
    pub fn resolve(
        &self,
        public_name: &PublicOpName,
        version: Option<OpVersion>,
    ) -> Result<Arc<dyn LaunchableOp>, EntryPointOpError> {
        match version {
            Some(version) => self.resolve_version(public_name, version),
            None => self.resolve_latest(public_name),
        }
    }

    /// Returns every registered public entry-point descriptor in deterministic order.
    pub fn registered_entry_points(&self) -> Vec<EntryPointDescriptor> {
        self.ops
            .values()
            .flat_map(BTreeMap::values)
            .map(|op| op.descriptor())
            .collect()
    }

    /// Resolves a public name to a specific registered version.
    pub fn resolve_version(
        &self,
        public_name: &PublicOpName,
        version: OpVersion,
    ) -> Result<Arc<dyn LaunchableOp>, EntryPointOpError> {
        let versions = self.ops.get(public_name).ok_or_else(|| {
            EntryPointOpError::new("EntryPointOpNotFound", "entry-point op is not registered")
        })?;
        versions.get(&version).map(Arc::clone).ok_or_else(|| {
            EntryPointOpError::new(
                "EntryPointOpVersionNotFound",
                "entry-point op version is not registered",
            )
        })
    }

    /// Returns the canonical digest of the registered public entry-point surface.
    pub fn registry_digest(&self) -> Result<ContentDigest, EntryPointOpError> {
        let mut entries = Vec::new();
        for (public_name, versions) in &self.ops {
            for (version, op) in versions {
                let descriptor = op.descriptor();
                let op_id = op.op_id();
                let mut formats = descriptor
                    .accepted_config_formats
                    .iter()
                    .map(|format| format.as_str())
                    .collect::<Vec<_>>();
                formats.sort_unstable();
                entries.push(serde_json::json!({
                    "accepted_config_formats": formats,
                    "op_id": {
                        "name": op_id.name.as_str(),
                        "namespace": op_id.namespace.as_str(),
                        "version": op_id.version.get(),
                    },
                    "public_name": public_name.as_str(),
                    "version": version.get(),
                }));
            }
        }
        let value = serde_json::json!({
            "entries": entries,
            "kind": "mfm.entry_point_op_registry.v1",
        });
        let json = serde_json::to_string(&value).map_err(|_| {
            EntryPointOpError::new(
                "EntryPointOpRegistryDigestFailed",
                "entry-point registry digest could not be serialized",
            )
        })?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map(|canonical| canonical.content_digest())
            .map_err(|_| {
                EntryPointOpError::new(
                    "EntryPointOpRegistryDigestFailed",
                    "entry-point registry digest could not be canonicalized",
                )
            })
    }
}

/// Error returned while resolving, registering, or planning entry-point operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct EntryPointOpError {
    code: String,
    message: String,
}

impl EntryPointOpError {
    /// Creates a public-safe entry-point operation error.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// Returns the stable error code.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the public-safe error message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<AuthoredConfigError> for EntryPointOpError {
    fn from(error: AuthoredConfigError) -> Self {
        Self::new(error.code().to_owned(), error.message().to_owned())
    }
}

#[cfg(test)]
#[path = "entry_point_tests.rs"]
mod tests;
