use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::Arc;

use mfm_authored_config::{
    AuthoredConfig, AuthoredConfigError, AuthoredConfigFormat, EntryPointDescriptor,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, NameToken, ResourceNamespace};
use mfm_program::{
    TypedProgramConfigMaterial, TypedProgramDraft, TypedProgramLaunchPlan, TypedProgramSeedMaterial,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Public entry-point operation name accepted by CLI and REST transports.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicOpName(NameToken);

impl PublicOpName {
    /// Creates a checked public operation name.
    pub fn new(value: impl AsRef<str>) -> Result<Self, EntryPointOpResolveError> {
        let value = value.as_ref();
        NameToken::new(value).map(Self).map_err(|error| {
            EntryPointOpResolveError::new(
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
    ) -> Result<Self, EntryPointOpResolveError> {
        let namespace = namespace.as_ref();
        let name = name.as_ref();
        let namespace = ResourceNamespace::new(namespace).map_err(|error| {
            EntryPointOpResolveError::new(
                "InvalidEntryPointOpId",
                format!("entry-point op namespace is invalid: {error}"),
            )
        })?;
        let name = NameToken::new(name).map_err(|error| {
            EntryPointOpResolveError::new(
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

/// Deterministic plan returned by a launchable entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryPointOpPlan {
    /// Typed program draft to certify before runtime launch.
    pub draft: TypedProgramDraft,
    /// Canonical config artifacts required by the draft.
    pub config_material: Vec<TypedProgramConfigMaterial>,
    /// Canonical seed artifacts required by the draft.
    pub seed_material: Vec<TypedProgramSeedMaterial>,
}

impl From<TypedProgramLaunchPlan> for EntryPointOpPlan {
    fn from(plan: TypedProgramLaunchPlan) -> Self {
        Self {
            draft: plan.draft,
            config_material: plan.config_material,
            seed_material: plan.seed_material,
        }
    }
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
    fn accepted_config_formats(&self) -> &'static [AuthoredConfigFormat];

    /// Deterministically plans the typed program draft and launch material.
    fn plan(&self, authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError>;
}

/// Generic app adapter from a typed op-crate planner to [`LaunchableOp`].
pub struct EntryPointPlannerAdapter<TConfig, E> {
    descriptor: EntryPointDescriptor,
    op_id: EntryPointOpId,
    public_name: PublicOpName,
    version: OpVersion,
    planner: fn(TConfig) -> Result<TypedProgramLaunchPlan, E>,
    map_error: fn(E) -> OpLaunchError,
    _config: PhantomData<fn() -> TConfig>,
}

impl<TConfig, E> EntryPointPlannerAdapter<TConfig, E> {
    /// Builds a launchable adapter from app-neutral descriptor and planner exports.
    pub fn new(
        descriptor: EntryPointDescriptor,
        planner: fn(TConfig) -> Result<TypedProgramLaunchPlan, E>,
        map_error: fn(E) -> OpLaunchError,
    ) -> Result<Self, EntryPointOpResolveError> {
        if descriptor.accepted_config_formats.is_empty() {
            return Err(EntryPointOpResolveError::new(
                "EntryPointOpConfigFormatsEmpty",
                "entry-point op must accept at least one config format",
            ));
        }
        let version = OpVersion::new(descriptor.version)?;
        Ok(Self {
            descriptor,
            op_id: EntryPointOpId::new(descriptor.namespace, descriptor.name, version)?,
            public_name: PublicOpName::new(descriptor.public_name)?,
            version,
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
    fn op_id(&self) -> EntryPointOpId {
        self.op_id.clone()
    }

    fn public_name(&self) -> PublicOpName {
        self.public_name.clone()
    }

    fn version(&self) -> OpVersion {
        self.version
    }

    fn accepted_config_formats(&self) -> &'static [AuthoredConfigFormat] {
        self.descriptor.accepted_config_formats
    }

    fn plan(&self, authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError> {
        if !self
            .accepted_config_formats()
            .contains(&authored_config.format())
        {
            return Err(OpLaunchError::new(
                "EntryPointOpConfigFormatUnsupported",
                "entry-point op does not accept the supplied config format",
            ));
        }

        let normalized = authored_config.normalize::<TConfig>()?;
        let planned = (self.planner)(normalized.value).map_err(self.map_error)?;
        Ok(planned.into())
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
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

/// Error returned while planning an entry-point operation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
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

impl From<AuthoredConfigError> for OpLaunchError {
    fn from(error: AuthoredConfigError) -> Self {
        Self::new(error.code().to_owned(), error.message().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static FORMATS: &[AuthoredConfigFormat] =
        &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json];

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

        fn accepted_config_formats(&self) -> &'static [AuthoredConfigFormat] {
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
            })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct AdapterConfig {
        value: u64,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct AdapterPlanError;

    static JSON_FORMAT: &[AuthoredConfigFormat] = &[AuthoredConfigFormat::Json];

    fn adapter_descriptor() -> EntryPointDescriptor {
        EntryPointDescriptor {
            namespace: "mfm.test",
            name: "planner_adapter",
            public_name: "planner_adapter",
            version: 1,
            accepted_config_formats: FORMATS,
        }
    }

    fn json_only_adapter_descriptor() -> EntryPointDescriptor {
        EntryPointDescriptor {
            namespace: "mfm.test",
            name: "planner_adapter",
            public_name: "planner_adapter",
            version: 1,
            accepted_config_formats: JSON_FORMAT,
        }
    }

    fn adapter_plan(config: AdapterConfig) -> Result<TypedProgramLaunchPlan, AdapterPlanError> {
        if config.value == 0 {
            return Err(AdapterPlanError);
        }
        let draft = mfm_op_proof::proof_program_draft(mfm_op_proof::ProofWorkflowConfig::default())
            .map_err(|_| AdapterPlanError)?;
        Ok(TypedProgramLaunchPlan {
            draft,
            config_material: Vec::new(),
            seed_material: Vec::new(),
        })
    }

    fn adapter_plan_error(_error: AdapterPlanError) -> OpLaunchError {
        OpLaunchError::new("AdapterPlanFailed", "adapter test plan failed")
    }

    #[test]
    fn entry_point_registry_resolves_latest_and_explicit_versions() {
        enum Case {
            Latest,
            Explicit,
        }

        let name = PublicOpName::new("portfolio_snapshot").expect("public name");
        let mut registry = EntryPointOpRegistry::new();
        registry
            .register(FakeOp::new("portfolio_snapshot", 1))
            .unwrap();
        registry
            .register(FakeOp::new("portfolio_snapshot", 2))
            .unwrap();

        for (case, expected_version) in [(Case::Latest, 2), (Case::Explicit, 1)] {
            let op = match case {
                Case::Latest => registry.resolve_latest(&name).expect("latest op"),
                Case::Explicit => registry
                    .resolve(&name, Some(OpVersion::new(1).unwrap()))
                    .expect("versioned op"),
            };

            assert_eq!(op.version().get(), expected_version);
        }
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
    fn launchable_op_plan_returns_draft_without_certification() {
        let op = FakeOp::new("portfolio_snapshot", 1);
        let plan = op
            .plan(
                AuthoredConfig::new(AuthoredConfigFormat::Toml, "portfolio_id = \"main\"\n")
                    .expect("authored config"),
            )
            .expect("plan");

        assert!(!plan.draft.state_nodes().is_empty());
        assert!(plan.config_material.is_empty());
    }

    #[test]
    fn entry_point_planner_adapter_plans_from_authored_config() {
        let op =
            EntryPointPlannerAdapter::new(adapter_descriptor(), adapter_plan, adapter_plan_error)
                .expect("adapter op");
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Json, r#"{"value":1}"#).expect("authored");

        let plan = op.plan(authored).expect("plan");

        assert_eq!(op.op_id().to_string(), "mfm.test:planner_adapter:1");
        assert_eq!(op.public_name().as_str(), "planner_adapter");
        assert!(!plan.draft.state_nodes().is_empty());
        assert!(plan.config_material.is_empty());
        assert!(plan.seed_material.is_empty());
    }

    #[test]
    fn entry_point_planner_adapter_rejects_unsupported_format_before_planning() {
        let op = EntryPointPlannerAdapter::new(
            json_only_adapter_descriptor(),
            adapter_plan,
            adapter_plan_error,
        )
        .expect("adapter op");
        let authored =
            AuthoredConfig::new(AuthoredConfigFormat::Toml, "value = 1").expect("authored");

        let err = op.plan(authored).expect_err("unsupported format");

        assert_eq!(err.code(), "EntryPointOpConfigFormatUnsupported");
    }
}
