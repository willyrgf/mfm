use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentDigest, DigestAlgorithm, SchemaId, SeedId};
use mfm_program::TypedProgramDraft;
use mfm_spec::v1 as spec;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default authored config format when callers do not provide one.
pub const DEFAULT_AUTHORED_CONFIG_FORMAT: ConfigFormat = ConfigFormat::Toml;

/// Maximum authored config payload size accepted before parsing.
pub const DEFAULT_AUTHORED_CONFIG_MAX_BYTES: usize = 256 * 1024;

/// Public entry-point operation name accepted by CLI and REST transports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicOpName(String);

impl PublicOpName {
    /// Creates a checked public operation name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, EntryPointOpResolveError> {
        let value = value.as_ref();
        validate_public_op_name(value)
            .map_err(|message| EntryPointOpResolveError::new("InvalidPublicOpName", message))?;
        Ok(Self(value.to_owned()))
    }

    /// Returns the public name as transport text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PublicOpName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PublicOpName {
    type Err = EntryPointOpResolveError;

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
    pub fn new(value: u32) -> Result<Self, EntryPointOpResolveError> {
        if value == 0 {
            return Err(EntryPointOpResolveError::new(
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
    type Err = EntryPointOpResolveError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed = value.parse::<u32>().map_err(|_| {
            EntryPointOpResolveError::new(
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
    pub namespace: String,
    /// Stable domain operation name within the namespace.
    pub name: String,
    /// Public operation version.
    pub version: OpVersion,
}

impl EntryPointOpId {
    /// Creates a checked entry-point operation id.
    pub fn new(
        namespace: impl AsRef<str>,
        name: impl AsRef<str>,
        version: OpVersion,
    ) -> Result<Self, EntryPointOpResolveError> {
        let namespace = namespace.as_ref();
        let name = name.as_ref();
        validate_namespace(namespace)
            .map_err(|message| EntryPointOpResolveError::new("InvalidEntryPointOpId", message))?;
        validate_public_op_name(name)
            .map_err(|message| EntryPointOpResolveError::new("InvalidEntryPointOpId", message))?;
        Ok(Self {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            version,
        })
    }
}

impl fmt::Display for EntryPointOpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.namespace, self.name, self.version)
    }
}

/// Authored config encoding accepted by a launchable entry-point operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFormat {
    /// TOML authored config.
    Toml,
    /// JSON authored config.
    Json,
}

impl ConfigFormat {
    /// Returns the stable transport spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Json => "json",
        }
    }
}

impl fmt::Display for ConfigFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ConfigFormat {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "toml" => Ok(Self::Toml),
            "json" => Ok(Self::Json),
            _ => Err("config format must be `toml` or `json`".to_owned()),
        }
    }
}

/// Public authored config bytes submitted to an entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthoredConfig {
    format: ConfigFormat,
    bytes: Vec<u8>,
    authored_digest: ContentDigest,
}

impl AuthoredConfig {
    /// Creates authored config using the default size limit.
    pub fn new(format: ConfigFormat, bytes: impl Into<Vec<u8>>) -> Result<Self, OpLaunchError> {
        Self::with_size_limit(format, bytes, DEFAULT_AUTHORED_CONFIG_MAX_BYTES)
    }

    /// Creates authored config using an explicit size limit.
    pub fn with_size_limit(
        format: ConfigFormat,
        bytes: impl Into<Vec<u8>>,
        max_bytes: usize,
    ) -> Result<Self, OpLaunchError> {
        let bytes = bytes.into();
        if bytes.len() > max_bytes {
            return Err(OpLaunchError::new(
                "AuthoredConfigTooLarge",
                "authored config exceeds the maximum accepted size",
            ));
        }
        let authored_digest =
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
        Ok(Self {
            format,
            bytes,
            authored_digest,
        })
    }

    /// Creates authored config from a REST-style JSON `config` value.
    ///
    /// String config values default to TOML when no format is supplied. JSON objects or arrays
    /// imply JSON unless an explicit JSON format is supplied. TOML cannot be represented as a
    /// structured JSON config value.
    pub fn from_json_transport_value(
        explicit_format: Option<ConfigFormat>,
        value: &Value,
    ) -> Result<Self, OpLaunchError> {
        match value {
            Value::String(raw) => Self::new(
                explicit_format.unwrap_or(DEFAULT_AUTHORED_CONFIG_FORMAT),
                raw.as_bytes(),
            ),
            Value::Object(_) | Value::Array(_) => {
                let format = explicit_format.unwrap_or(ConfigFormat::Json);
                if format != ConfigFormat::Json {
                    return Err(OpLaunchError::new(
                        "AuthoredConfigFormatShapeMismatch",
                        "structured JSON config values require json config format",
                    ));
                }
                let bytes = serde_json::to_vec(value).map_err(|_| {
                    OpLaunchError::new(
                        "AuthoredConfigSerializeFailed",
                        "authored config could not be serialized",
                    )
                })?;
                Self::new(ConfigFormat::Json, bytes)
            }
            _ => Err(OpLaunchError::new(
                "AuthoredConfigShapeInvalid",
                "authored config must be a string or structured JSON value",
            )),
        }
    }

    /// Returns the authored config format.
    pub const fn format(&self) -> ConfigFormat {
        self.format
    }

    /// Returns the original authored config bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the digest of the original authored config bytes.
    pub fn authored_digest(&self) -> &ContentDigest {
        &self.authored_digest
    }

    /// Parses, validates, and canonicalizes this authored config into a typed value.
    pub fn normalize<T>(&self) -> Result<NormalizedAuthoredConfig<T>, OpLaunchError>
    where
        T: DeserializeOwned + Serialize,
    {
        normalize_authored_config(self)
    }
}

/// Typed config plus canonical evidence derived from authored config bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedAuthoredConfig<T> {
    /// Decoded typed config.
    pub value: T,
    /// Effective config format used to parse the authored bytes.
    pub format: ConfigFormat,
    /// Digest of the original authored bytes.
    pub authored_digest: ContentDigest,
    /// Canonical JSON bytes for the decoded typed config.
    pub canonical_json: PlainCanonicalJsonBytes,
    /// Digest of the canonical JSON bytes.
    pub canonical_digest: ContentDigest,
}

/// Canonical non-secret config material emitted by entry-point operation planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalConfigMaterial {
    /// Config schema id consumed by the certified typed spec.
    pub schema_id: SchemaId,
    /// Canonical JSON config bytes.
    pub bytes: PlainCanonicalJsonBytes,
    /// Media type for the canonical config bytes.
    pub media_type: spec::MediaType,
}

/// Canonical non-secret seed material emitted by entry-point operation planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSeedMaterial {
    /// Seed id consumed by the certified typed spec.
    pub seed_id: SeedId,
    /// Canonical JSON seed bytes.
    pub bytes: PlainCanonicalJsonBytes,
    /// Media type for the canonical seed bytes.
    pub media_type: spec::MediaType,
}

/// Stable identity for the deterministic lowering used by an entry-point plan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LoweringIdentity(String);

impl LoweringIdentity {
    /// Creates a checked lowering identity.
    pub fn new(value: impl AsRef<str>) -> Result<Self, OpLaunchError> {
        checked_identity("lowering identity", value.as_ref()).map(Self)
    }

    /// Returns the identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LoweringIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable identity for the config canonicalizer used by an entry-point plan.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalizerIdentity(String);

impl CanonicalizerIdentity {
    /// Creates a checked canonicalizer identity.
    pub fn new(value: impl AsRef<str>) -> Result<Self, OpLaunchError> {
        checked_identity("canonicalizer identity", value.as_ref()).map(Self)
    }

    /// Returns the identity string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CanonicalizerIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Deterministic plan returned by a launchable entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointOpPlan {
    /// Typed program draft to certify before runtime launch.
    pub draft: TypedProgramDraft,
    /// Canonical config artifacts required by the draft.
    pub config_material: Vec<CanonicalConfigMaterial>,
    /// Canonical seed artifacts required by the draft.
    pub seed_material: Vec<CanonicalSeedMaterial>,
    /// Public output schema id exposed by the op, when the workflow has one.
    pub public_output_schema_id: Option<SchemaId>,
    /// Deterministic lowering identity that produced the draft.
    pub lowering_identity: LoweringIdentity,
    /// Config canonicalizer identity that produced config material.
    pub canonicalizer_identity: CanonicalizerIdentity,
    /// Digest of the original authored config bytes.
    pub authored_config_digest: ContentDigest,
    /// Digest of the canonical op config bytes.
    pub canonical_config_digest: ContentDigest,
}

/// Operation that can plan a public entry-point run from authored config.
pub trait LaunchableOp: Send + Sync {
    /// Returns the stable entry-point operation id.
    fn op_id(&self) -> EntryPointOpId;

    /// Returns the public shorthand name.
    fn public_name(&self) -> PublicOpName;

    /// Returns the public version selector.
    fn version(&self) -> OpVersion;

    /// Returns authored config formats accepted by this operation.
    fn accepted_config_formats(&self) -> &'static [ConfigFormat];

    /// Deterministically plans the typed program draft and launch material.
    fn plan(&self, authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError>;
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
    pub fn register(
        &mut self,
        op: impl LaunchableOp + 'static,
    ) -> Result<(), EntryPointOpResolveError> {
        self.register_arc(Arc::new(op))
    }

    /// Registers an already shared launchable operation.
    pub fn register_arc(
        &mut self,
        op: Arc<dyn LaunchableOp>,
    ) -> Result<(), EntryPointOpResolveError> {
        if op.accepted_config_formats().is_empty() {
            return Err(EntryPointOpResolveError::new(
                "EntryPointOpConfigFormatsEmpty",
                "entry-point op must accept at least one config format",
            ));
        }
        let public_name = op.public_name();
        let version = op.version();
        let op_id = op.op_id();
        if op_id.version != version {
            return Err(EntryPointOpResolveError::new(
                "EntryPointOpVersionMismatch",
                "entry-point op id version must match registered version",
            ));
        }

        let versions = self.ops.entry(public_name).or_default();
        if versions.contains_key(&version) {
            return Err(EntryPointOpResolveError::new(
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
    ) -> Result<Arc<dyn LaunchableOp>, EntryPointOpResolveError> {
        let versions = self.ops.get(public_name).ok_or_else(|| {
            EntryPointOpResolveError::new(
                "EntryPointOpNotFound",
                "entry-point op is not registered",
            )
        })?;
        versions
            .last_key_value()
            .map(|(_version, op)| Arc::clone(op))
            .ok_or_else(|| {
                EntryPointOpResolveError::new(
                    "EntryPointOpNotFound",
                    "entry-point op is not registered",
                )
            })
    }

    /// Resolves a public name and optional explicit version.
    pub fn resolve(
        &self,
        public_name: &PublicOpName,
        version: Option<OpVersion>,
    ) -> Result<Arc<dyn LaunchableOp>, EntryPointOpResolveError> {
        match version {
            Some(version) => self.resolve_version(public_name, version),
            None => self.resolve_latest(public_name),
        }
    }

    /// Resolves a public name to a specific registered version.
    pub fn resolve_version(
        &self,
        public_name: &PublicOpName,
        version: OpVersion,
    ) -> Result<Arc<dyn LaunchableOp>, EntryPointOpResolveError> {
        let versions = self.ops.get(public_name).ok_or_else(|| {
            EntryPointOpResolveError::new(
                "EntryPointOpNotFound",
                "entry-point op is not registered",
            )
        })?;
        versions.get(&version).map(Arc::clone).ok_or_else(|| {
            EntryPointOpResolveError::new(
                "EntryPointOpVersionNotFound",
                "entry-point op version is not registered",
            )
        })
    }

    /// Returns the number of registered public name/version pairs.
    pub fn len(&self) -> usize {
        self.ops.values().map(BTreeMap::len).sum()
    }

    /// Returns whether the registry has no launchable operations.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Returns the canonical digest of the registered public entry-point surface.
    pub fn registry_digest(&self) -> Result<ContentDigest, EntryPointOpResolveError> {
        let mut entries = Vec::new();
        for (public_name, versions) in &self.ops {
            for (version, op) in versions {
                let op_id = op.op_id();
                let mut formats = op
                    .accepted_config_formats()
                    .iter()
                    .map(|format| format.as_str())
                    .collect::<Vec<_>>();
                formats.sort_unstable();
                entries.push(serde_json::json!({
                    "accepted_config_formats": formats,
                    "op_id": {
                        "name": op_id.name,
                        "namespace": op_id.namespace,
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
            EntryPointOpResolveError::new(
                "EntryPointOpRegistryDigestFailed",
                "entry-point registry digest could not be serialized",
            )
        })?;
        PlainCanonicalJsonBytes::from_json_str(&json)
            .map(|canonical| canonical.content_digest())
            .map_err(|_| {
                EntryPointOpResolveError::new(
                    "EntryPointOpRegistryDigestFailed",
                    "entry-point registry digest could not be canonicalized",
                )
            })
    }
}

/// Error returned while resolving or registering entry-point operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointOpResolveError {
    code: String,
    message: String,
}

impl EntryPointOpResolveError {
    /// Creates a public-safe entry-point op resolution error.
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

impl fmt::Display for EntryPointOpResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EntryPointOpResolveError {}

/// Error returned while planning an entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpLaunchError {
    code: String,
    message: String,
}

impl OpLaunchError {
    /// Creates a public-safe entry-point operation planning error.
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

impl fmt::Display for OpLaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for OpLaunchError {}

fn normalize_authored_config<T>(
    authored: &AuthoredConfig,
) -> Result<NormalizedAuthoredConfig<T>, OpLaunchError>
where
    T: DeserializeOwned + Serialize,
{
    let submitted = parse_authored_value(authored)?;
    let value = serde_json::from_value::<T>(submitted.clone()).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigDecodeFailed",
            "authored config does not match the selected op schema",
        )
    })?;
    let normalized = serde_json::to_value(&value).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigSerializeFailed",
            "authored config could not be serialized",
        )
    })?;
    reject_unknown_submitted_fields(&submitted, &normalized)?;
    reject_json_floats(&normalized)?;
    let canonical_json = canonical_json_value(&normalized)?;
    let canonical_digest = canonical_json.content_digest();
    Ok(NormalizedAuthoredConfig {
        value,
        format: authored.format(),
        authored_digest: authored.authored_digest().clone(),
        canonical_json,
        canonical_digest,
    })
}

fn parse_authored_value(authored: &AuthoredConfig) -> Result<Value, OpLaunchError> {
    match authored.format() {
        ConfigFormat::Json => parse_json_authored_value(authored.bytes()),
        ConfigFormat::Toml => parse_toml_authored_value(authored.bytes()),
    }
}

fn parse_json_authored_value(bytes: &[u8]) -> Result<Value, OpLaunchError> {
    let raw = std::str::from_utf8(bytes).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigInvalidUtf8",
            "authored config must be UTF-8 text",
        )
    })?;
    let loose = serde_json::from_str::<Value>(raw).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigInvalidJson",
            "authored config is not valid JSON",
        )
    })?;
    reject_json_floats(&loose)?;
    let canonical = PlainCanonicalJsonBytes::from_json_str(raw).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigInvalidJson",
            "authored config is not valid duplicate-free canonicalizable JSON",
        )
    })?;
    json_value_from_canonical(canonical.as_bytes())
}

fn parse_toml_authored_value(bytes: &[u8]) -> Result<Value, OpLaunchError> {
    let raw = std::str::from_utf8(bytes).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigInvalidUtf8",
            "authored config must be UTF-8 text",
        )
    })?;
    let toml_value = raw.parse::<toml::Value>().map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigInvalidToml",
            "authored config is not valid TOML",
        )
    })?;
    let json_value = serde_json::to_value(toml_value).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigSerializeFailed",
            "authored config could not be serialized",
        )
    })?;
    reject_json_floats(&json_value)?;
    let canonical = canonical_json_value(&json_value)?;
    json_value_from_canonical(canonical.as_bytes())
}

fn canonical_json_value(value: &Value) -> Result<PlainCanonicalJsonBytes, OpLaunchError> {
    let json = serde_json::to_string(value).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigSerializeFailed",
            "authored config could not be serialized",
        )
    })?;
    PlainCanonicalJsonBytes::from_json_str(&json).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigCanonicalJsonFailed",
            "authored config could not be canonicalized",
        )
    })
}

fn json_value_from_canonical(bytes: &[u8]) -> Result<Value, OpLaunchError> {
    serde_json::from_slice(bytes).map_err(|_| {
        OpLaunchError::new(
            "AuthoredConfigCanonicalJsonFailed",
            "authored config could not be canonicalized",
        )
    })
}

fn reject_json_floats(value: &Value) -> Result<(), OpLaunchError> {
    match value {
        Value::Number(number) if !(number.is_i64() || number.is_u64()) => Err(OpLaunchError::new(
            "AuthoredConfigFloatUnsupported",
            "authored config must not contain floating point numbers",
        )),
        Value::Array(values) => {
            for value in values {
                reject_json_floats(value)?;
            }
            Ok(())
        }
        Value::Object(entries) => {
            for value in entries.values() {
                reject_json_floats(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn reject_unknown_submitted_fields(
    submitted: &Value,
    normalized: &Value,
) -> Result<(), OpLaunchError> {
    match (submitted, normalized) {
        (Value::Object(submitted), Value::Object(normalized)) => {
            for (key, submitted_value) in submitted {
                let Some(normalized_value) = normalized.get(key) else {
                    return Err(unknown_authored_config_field());
                };
                reject_unknown_submitted_fields(submitted_value, normalized_value)?;
            }
            Ok(())
        }
        (Value::Array(submitted), Value::Array(normalized)) => {
            if submitted.len() != normalized.len() {
                return Err(unknown_authored_config_field());
            }
            for (submitted_value, normalized_value) in submitted.iter().zip(normalized) {
                reject_unknown_submitted_fields(submitted_value, normalized_value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn unknown_authored_config_field() -> OpLaunchError {
    OpLaunchError::new(
        "AuthoredConfigUnknownField",
        "authored config contains fields outside the selected op schema",
    )
}

fn validate_public_op_name(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("public op name must not be empty".to_owned());
    }
    let mut previous_underscore = false;
    for (index, byte) in value.bytes().enumerate() {
        let valid =
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'_');
        if !valid {
            return Err(
                "public op name must use lowercase ASCII letters, digits, and underscores"
                    .to_owned(),
            );
        }
        if index == 0 && !byte.is_ascii_lowercase() {
            return Err("public op name must start with a lowercase ASCII letter".to_owned());
        }
        if byte == b'_' {
            if previous_underscore {
                return Err("public op name must not contain consecutive underscores".to_owned());
            }
            previous_underscore = true;
        } else {
            previous_underscore = false;
        }
    }
    if value.ends_with('_') {
        return Err("public op name must not end with an underscore".to_owned());
    }
    Ok(())
}

fn validate_namespace(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("entry-point op namespace must not be empty".to_owned());
    }
    for segment in value.split('.') {
        if segment.is_empty() {
            return Err("entry-point op namespace must not contain empty segments".to_owned());
        }
        validate_namespace_segment(segment)?;
    }
    Ok(())
}

fn validate_namespace_segment(value: &str) -> Result<(), String> {
    for (index, byte) in value.bytes().enumerate() {
        let valid =
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-');
        if !valid {
            return Err(
                "entry-point op namespace must use lowercase ASCII letters, digits, dots, and dashes"
                    .to_owned(),
            );
        }
        if index == 0 && !byte.is_ascii_lowercase() {
            return Err(
                "entry-point op namespace segments must start with a lowercase ASCII letter"
                    .to_owned(),
            );
        }
    }
    if value.ends_with('-') {
        return Err("entry-point op namespace segments must not end with a dash".to_owned());
    }
    Ok(())
}

fn checked_identity(label: &'static str, value: &str) -> Result<String, OpLaunchError> {
    if value.is_empty() {
        return Err(OpLaunchError::new(
            "InvalidEntryPointOpIdentity",
            format!("{label} must not be empty"),
        ));
    }
    if value.bytes().any(|byte| {
        !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(OpLaunchError::new(
            "InvalidEntryPointOpIdentity",
            format!(
                "{label} must use lowercase ASCII letters, digits, dots, underscores, or dashes"
            ),
        ));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    static FORMATS: &[ConfigFormat] = &[ConfigFormat::Toml, ConfigFormat::Json];

    #[derive(Clone)]
    struct FakeOp {
        public_name: PublicOpName,
        version: OpVersion,
    }

    impl FakeOp {
        fn new(public_name: &str, version: u32) -> Self {
            Self {
                public_name: PublicOpName::new(public_name).expect("public name"),
                version: OpVersion::new(version).expect("version"),
            }
        }
    }

    impl LaunchableOp for FakeOp {
        fn op_id(&self) -> EntryPointOpId {
            EntryPointOpId::new("mfm.test", self.public_name.as_str(), self.version).expect("op id")
        }

        fn public_name(&self) -> PublicOpName {
            self.public_name.clone()
        }

        fn version(&self) -> OpVersion {
            self.version
        }

        fn accepted_config_formats(&self) -> &'static [ConfigFormat] {
            FORMATS
        }

        fn plan(
            &self,
            _authored_config: AuthoredConfig,
        ) -> Result<EntryPointOpPlan, OpLaunchError> {
            let draft =
                mfm_op_proof::proof_program_draft(mfm_op_proof::ProofWorkflowConfig::default())
                    .map_err(|error| {
                        OpLaunchError::new("EntryPointOpPlanFailed", error.to_string())
                    })?;
            Ok(EntryPointOpPlan {
                draft,
                config_material: Vec::new(),
                seed_material: Vec::new(),
                public_output_schema_id: None,
                lowering_identity: LoweringIdentity::new("mfm.test.lowering.v1")?,
                canonicalizer_identity: CanonicalizerIdentity::new("mfm.test.canonicalizer.v1")?,
                authored_config_digest: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    mfm_canonical::sha256_digest_bytes(b"authored"),
                ),
                canonical_config_digest: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    mfm_canonical::sha256_digest_bytes(b"canonical"),
                ),
            })
        }
    }

    #[test]
    fn entry_point_registry_resolves_latest_version() {
        let name = PublicOpName::new("portfolio_snapshot").expect("public name");
        let mut registry = EntryPointOpRegistry::new();
        registry
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();
        registry
            .register(FakeOp::new("portfolio_snapshot", 2))
            .unwrap();

        let op = registry.resolve_latest(&name).expect("latest op");

        assert_eq!(op.version().get(), 2);
    }

    #[test]
    fn entry_point_registry_resolves_explicit_version() {
        let name = PublicOpName::new("portfolio_snapshot").expect("public name");
        let mut registry = EntryPointOpRegistry::new();
        registry
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();
        registry
            .register(FakeOp::new("portfolio_snapshot", 2))
            .unwrap();

        let op = registry
            .resolve(&name, Some(OpVersion::new(1).unwrap()))
            .expect("versioned op");

        assert_eq!(op.version().get(), 1);
    }

    #[test]
    fn entry_point_registry_rejects_duplicate_public_name_and_version() {
        let mut registry = EntryPointOpRegistry::new();
        registry
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();

        let err = registry
            .register(FakeOp::new("portfolio_snapshot", 1))
            .expect_err("duplicate rejects");

        assert_eq!(err.code(), "DuplicateEntryPointOp");
    }

    #[test]
    fn entry_point_registry_reports_unknown_name_and_version() {
        let mut registry = EntryPointOpRegistry::new();
        registry
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();
        let missing_name = PublicOpName::new("evm_contract_lifecycle").unwrap();
        let portfolio = PublicOpName::new("portfolio_snapshot").unwrap();

        let missing = match registry.resolve_latest(&missing_name) {
            Ok(_) => panic!("missing op must reject"),
            Err(error) => error,
        };
        let missing_version = match registry.resolve_version(&portfolio, OpVersion::new(2).unwrap())
        {
            Ok(_) => panic!("missing version must reject"),
            Err(error) => error,
        };

        assert_eq!(missing.code(), "EntryPointOpNotFound");
        assert_eq!(missing_version.code(), "EntryPointOpVersionNotFound");
    }

    #[test]
    fn entry_point_registry_digest_is_deterministic_and_surface_bound() {
        let mut first = EntryPointOpRegistry::new();
        first
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();
        first
            .register(FakeOp::new("portfolio_snapshot", 2))
            .unwrap();
        let mut same = EntryPointOpRegistry::new();
        same.register(FakeOp::new("portfolio_snapshot", 2)).unwrap();
        same.register(FakeOp::new("portfolio_snapshot", 1)).unwrap();
        let mut different = EntryPointOpRegistry::new();
        different
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();

        assert_eq!(
            first.registry_digest().unwrap(),
            same.registry_digest().unwrap()
        );
        assert_ne!(
            first.registry_digest().unwrap(),
            different.registry_digest().unwrap()
        );
    }

    #[test]
    fn public_op_name_uses_lowercase_snake_case() {
        assert!(PublicOpName::new("evm_contract_lifecycle").is_ok());
        assert!(PublicOpName::new("EvmContractLifecycle").is_err());
        assert!(PublicOpName::new("evm__contract").is_err());
        assert!(PublicOpName::new("evm_contract_").is_err());
    }

    #[test]
    fn launchable_op_plan_returns_draft_without_certification() {
        let op = FakeOp::new("portfolio_snapshot", 1);
        let plan = op
            .plan(
                AuthoredConfig::new(ConfigFormat::Toml, "portfolio_id = \"main\"\n")
                    .expect("authored config"),
            )
            .expect("plan");

        assert!(!plan.draft.state_nodes().is_empty());
        assert!(plan.config_material.is_empty());
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct SampleConfig {
        portfolio_id: String,
        quote_codes: Vec<String>,
        nested: Option<NestedConfig>,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct NestedConfig {
        count: u64,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct LenientConfig {
        portfolio_id: String,
    }

    #[test]
    fn authored_config_rest_string_defaults_to_toml() {
        let value = Value::String("portfolio_id = \"main\"\nquote_codes = [\"USD\"]\n".to_owned());
        let authored = AuthoredConfig::from_json_transport_value(None, &value).expect("authored");

        assert_eq!(authored.format(), ConfigFormat::Toml);
    }

    #[test]
    fn authored_config_rest_json_object_implies_json() {
        let value = serde_json::json!({
            "portfolio_id": "main",
            "quote_codes": ["USD"],
            "nested": null
        });
        let authored = AuthoredConfig::from_json_transport_value(None, &value).expect("authored");

        assert_eq!(authored.format(), ConfigFormat::Json);
    }

    #[test]
    fn authored_config_normalizes_toml_to_canonical_json_and_digests() {
        let authored = AuthoredConfig::new(
            ConfigFormat::Toml,
            "quote_codes = [\"USD\"]\nportfolio_id = \"main\"\n\n[nested]\ncount = 1\n",
        )
        .expect("authored");

        let normalized = authored
            .normalize::<SampleConfig>()
            .expect("normalized config");

        assert_eq!(normalized.format, ConfigFormat::Toml);
        assert_eq!(normalized.authored_digest, *authored.authored_digest());
        assert_eq!(
            normalized.canonical_json.as_str(),
            r#"{"nested":{"count":1},"portfolio_id":"main","quote_codes":["USD"]}"#
        );
        assert_eq!(
            normalized.canonical_digest,
            normalized.canonical_json.content_digest()
        );
    }

    #[test]
    fn authored_config_rejects_json_duplicate_keys() {
        let authored = AuthoredConfig::new(
            ConfigFormat::Json,
            r#"{"portfolio_id":"main","portfolio_id":"other"}"#,
        )
        .expect("authored");

        let err = authored
            .normalize::<LenientConfig>()
            .expect_err("duplicate");

        assert_eq!(err.code(), "AuthoredConfigInvalidJson");
    }

    #[test]
    fn authored_config_rejects_unknown_fields_by_default() {
        let authored = AuthoredConfig::new(
            ConfigFormat::Json,
            r#"{"portfolio_id":"main","unexpected":"value"}"#,
        )
        .expect("authored");

        let err = authored.normalize::<LenientConfig>().expect_err("unknown");

        assert_eq!(err.code(), "AuthoredConfigUnknownField");
    }

    #[test]
    fn authored_config_rejects_floats_before_hashing() {
        let authored =
            AuthoredConfig::new(ConfigFormat::Json, r#"{"amount":1.25}"#).expect("authored");

        let err = authored.normalize::<Value>().expect_err("float");

        assert_eq!(err.code(), "AuthoredConfigFloatUnsupported");
    }

    #[test]
    fn authored_config_rejects_oversized_input_before_parsing() {
        let err = AuthoredConfig::with_size_limit(ConfigFormat::Toml, "portfolio_id = \"main\"", 4)
            .expect_err("oversized");

        assert_eq!(err.code(), "AuthoredConfigTooLarge");
    }

    #[test]
    fn authored_config_parse_errors_are_redacted() {
        let authored = AuthoredConfig::new(
            ConfigFormat::Toml,
            "password = \"supersecret\"\nmnemonic = \"abandon abandon\"\n[",
        )
        .expect("authored");

        let err = authored
            .normalize::<LenientConfig>()
            .expect_err("invalid toml");

        assert_eq!(err.code(), "AuthoredConfigInvalidToml");
        assert!(!err.message().contains("supersecret"));
        assert!(!err.message().contains("abandon"));
        assert!(!err.message().contains("password"));
        assert!(!err.message().contains("mnemonic"));
    }
}
