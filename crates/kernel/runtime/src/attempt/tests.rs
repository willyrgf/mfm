use super::*;
use mfm_capabilities::CapabilitySetDescriptor;
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EffectKind,
    NodeId, SchemaId, ScopeId, StateKind, StateVersion,
};
use mfm_spec::v1::ResourceNamespace;

#[test]
fn resource_lane_block_detection_uses_typed_commit_outcome() {
    let node_id = node_id(7);

    let witness = resource_lane_block_witness_from_outcome(
        &node_id,
        store::WaitFifoAdmissionBlock {
            resource_lane_key: resource_lane_key("wallet-1"),
            holder: None,
            waiter: None,
        },
    );
    assert_eq!(witness.node_id, node_id);
    assert_eq!(
        witness.lane_key.namespace,
        ResourceNamespace::new("mfm.test.account_nonce").expect("namespace")
    );
    assert_eq!(
        witness.lane_key.key,
        events::ResourceKey::new("wallet-1").expect("resource key")
    );
}

#[test]
fn resource_lane_block_witness_does_not_block_unstarted_same_namespace_node() {
    let witness = ResourceLaneBlockWitness {
        node_id: node_id(7),
        lane_key: resource_lane_key("wallet-1"),
        waiter_id: None,
        lane_ticket: None,
    };
    let unrelated = exclusive_resource_node(node_id(8));

    assert!(!witness.blocks_node(&store::ProjectionSnapshot::default(), &unrelated));
}

fn resource_lane_key(key: &str) -> store::ResourceLaneKey {
    store::ResourceLaneKey {
        namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
        key_schema_id: schema_id(0x7b),
        key: events::ResourceKey::new(key).expect("resource key"),
    }
}

fn exclusive_resource_node(node_id: NodeId) -> spec::NodeSpec {
    let schema = schema_id(0x31);
    spec::NodeSpec {
        node_id,
        stable_key: spec::StableAuthorKey::new("exclusive-node").expect("stable key"),
        scope_id: ScopeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x32; 32]),
        ),
        state_kind: StateKind::new(
            "mfm.test",
            "exclusive",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x33; 32]),
        )
        .expect("state kind"),
        state_version: StateVersion::new("mfm.test.exclusive.v1").expect("state version"),
        descriptor_id: DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x34; 32]),
        ),
        context: spec::NodeContextSpec::no_context(),
        config_ref: spec::ConfigRef {
            schema_id: schema.clone(),
            artifact_id: ArtifactId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x35; 32]),
            ),
            digest: content_digest(0x36),
            byte_len: 2,
            media_type: spec::MediaType::new("application/json").expect("media"),
        },
        input_bindings: spec::InputBindingSpec {
            input_schema_id: schema.clone(),
            input_descriptor_id: DescriptorId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x37; 32]),
            ),
            root: spec::InputBindingNodeSpec::Unit,
            digest: content_digest(0x38),
        },
        output_cell: CellId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x39; 32]),
        ),
        effect_kind: EffectKind::new(
            "mfm.test",
            "side-effect",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x3a; 32]),
        )
        .expect("effect kind"),
        capability_bindings: CapabilitySetDescriptor::new(Vec::new()).expect("capabilities"),
        adapter_bindings: Vec::new(),
        side_effect: Some(spec::SideEffectContractSpec {
            contract_digest: content_digest(0x3b),
            resource_claim: spec::ResourceClaimSpec::Exclusive {
                namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
                key_schema: schema,
            },
            verification: spec::SideEffectVerificationSpec::Receipt,
        }),
        framework: None,
        fact_descriptor_allowlist: Vec::new(),
        planning_lineage: spec::PlanningLineage {
            active_operation_instances: Vec::new(),
            completed_operation_frames: Vec::new(),
            lineage_digest: content_digest(0x3c),
        },
        deterministic_predecessors: Vec::new(),
    }
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

fn schema_id(byte: u8) -> SchemaId {
    SchemaId::new(
        "mfm.test.schema",
        &format!("{byte}"),
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
    .expect("schema id")
}

fn content_digest(byte: u8) -> mfm_ids::ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}

#[test]
fn observed_failure_retryability_follows_policy_and_failure_class() {
    let retry_materialization = ObservedFailureRetryabilityPolicy {
        input_materialization_retryable: true,
    };
    let terminal_materialization = ObservedFailureRetryabilityPolicy {
        input_materialization_retryable: false,
    };

    assert!(retry_materialization.retryable_for(ObservedFailureClass::InputMaterialization));
    assert!(!terminal_materialization.retryable_for(ObservedFailureClass::InputMaterialization));
    assert!(!retry_materialization.retryable_for(ObservedFailureClass::InvalidRunnerOutput));
    assert!(!retry_materialization.retryable_for(ObservedFailureClass::RuntimeValidation));
}

#[test]
fn typed_runner_failure_metadata_controls_attempt_error() {
    let failure = crate::RuntimeFailure::new(
        events::ErrorCode::new("missing_fact").expect("code"),
        events::ErrorCategory::Validation,
        "missing_fact: no acceptable Platform fact",
        Vec::new(),
    )
    .expect("typed failure");
    let error = RuntimeError::Failure(failure);
    let info = observed_attempt_failure_info(
        &error,
        ObservedFailureRetryabilityPolicy {
            input_materialization_retryable: false,
        },
    )
    .expect("map failure")
    .expect("observed failure");
    assert_eq!(info.error.code.as_str(), "missing_fact");
    assert!(info.error.safe_message.contains("missing_fact"));

    let arbitrary_diagnostic = mfm_capabilities::RedactedProviderDiagnostic::new(
        mfm_ids::LocalPublicId::new("portfolio").expect("provider"),
        mfm_capabilities::ProviderDiagnosticCode::ResponseInvalid,
    )
    .with_field(
        mfm_ids::LocalPublicId::new("domain_code").expect("field"),
        mfm_capabilities::ProviderDiagnosticValue::Id(
            mfm_ids::LocalPublicId::new("different_code").expect("value"),
        ),
    );
    let failure = crate::RuntimeFailure::new(
        events::ErrorCode::new("missing_fact").expect("code"),
        events::ErrorCategory::Validation,
        "missing_fact: no acceptable Platform fact",
        vec![arbitrary_diagnostic],
    )
    .expect("typed failure");
    let info = observed_attempt_failure_info(
        &RuntimeError::Failure(failure),
        ObservedFailureRetryabilityPolicy {
            input_materialization_retryable: false,
        },
    )
    .expect("map failure")
    .expect("observed failure");
    assert_eq!(info.error.code.as_str(), "missing_fact");

    let generic = RuntimeError::InvalidRunnerOutput("not a domain error".to_owned());
    let info = observed_attempt_failure_info(
        &generic,
        ObservedFailureRetryabilityPolicy {
            input_materialization_retryable: false,
        },
    )
    .expect("map failure")
    .expect("observed failure");
    assert_eq!(info.error.code.as_str(), "runner_output_invalid");
}
