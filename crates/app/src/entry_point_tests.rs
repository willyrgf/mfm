use super::*;
use mfm_authored_config::AuthoredConfigFormat;

static FORMATS: &[AuthoredConfigFormat] = &[AuthoredConfigFormat::Toml, AuthoredConfigFormat::Json];

#[derive(Clone)]
struct FakeOp {
    descriptor: EntryPointDescriptor,
}

impl FakeOp {
    fn new(public_name: &'static str, version: u32) -> Self {
        Self {
            descriptor: EntryPointDescriptor {
                namespace: "mfm.test",
                name: "fake_op",
                public_name,
                version,
                accepted_config_formats: FORMATS,
            },
        }
    }
}

impl LaunchableOp for FakeOp {
    fn descriptor(&self) -> EntryPointDescriptor {
        self.descriptor
    }

    fn op_id(&self) -> EntryPointOpId {
        let version = OpVersion::new(self.descriptor.version).expect("version");
        EntryPointOpId::new("mfm.test", self.descriptor.name, version).expect("op id")
    }

    fn plan(&self, _authored_config: AuthoredConfig) -> Result<EntryPointOpPlan, OpLaunchError> {
        let draft = mfm_op_proof::proof_program_draft(mfm_op_proof::ProofWorkflowConfig::default())
            .map_err(|error| OpLaunchError::new("EntryPointOpPlanFailed", error.to_string()))?;
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

        assert_eq!(op.descriptor().version, expected_version);
    }
}

#[test]
fn entry_point_registry_lists_registered_descriptors_in_name_version_order() {
    let mut registry = EntryPointOpRegistry::new();
    registry
        .register(FakeOp::new("portfolio_snapshot", 2))
        .unwrap();
    registry
        .register(FakeOp::new("portfolio_snapshot", 1))
        .unwrap();
    registry
        .register(FakeOp::new("evm_native_balance", 1))
        .unwrap();

    let descriptors = registry.registered_entry_points();

    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| (descriptor.public_name, descriptor.version))
            .collect::<Vec<_>>(),
        vec![
            ("evm_native_balance", 1),
            ("portfolio_snapshot", 1),
            ("portfolio_snapshot", 2),
        ]
    );
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
    let missing_version = match registry.resolve_version(&portfolio, OpVersion::new(2).unwrap()) {
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
    let op = EntryPointPlannerAdapter::new(adapter_descriptor(), adapter_plan, adapter_plan_error)
        .expect("adapter op");
    let authored =
        AuthoredConfig::new(AuthoredConfigFormat::Json, r#"{"value":1}"#).expect("authored");

    let plan = op.plan(authored).expect("plan");

    assert_eq!(op.op_id().to_string(), "mfm.test:planner_adapter:1");
    assert_eq!(op.descriptor().public_name, "planner_adapter");
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
    let authored = AuthoredConfig::new(AuthoredConfigFormat::Toml, "value = 1").expect("authored");

    let err = op.plan(authored).expect_err("unsupported format");

    assert_eq!(err.code(), "EntryPointOpConfigFormatUnsupported");
}
