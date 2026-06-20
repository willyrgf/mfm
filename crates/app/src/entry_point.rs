use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{SchemaId, SeedId};
use mfm_program::TypedProgramDraft;
use mfm_spec::v1 as spec;
use serde::{Deserialize, Serialize};

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
}

impl AuthoredConfig {
    /// Creates authored config from an explicit format and byte payload.
    pub fn new(format: ConfigFormat, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            format,
            bytes: bytes.into(),
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
            .plan(AuthoredConfig::new(
                ConfigFormat::Toml,
                "portfolio_id = \"main\"\n",
            ))
            .expect("plan");

        assert!(!plan.draft.state_nodes().is_empty());
        assert!(plan.config_material.is_empty());
    }
}
