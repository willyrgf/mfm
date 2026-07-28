use super::*;
use mfm_events::v1 as events;
use mfm_ids::{DigestAlgorithm, DigestBytes, NodeId, SchemaId};
use mfm_spec::v1::ResourceNamespace;

#[test]
fn resource_lane_block_detection_uses_typed_commit_outcome() {
    let node_id = node_id(7);

    let witness = resource_lane_block_witness_from_outcome(
        &node_id,
        &store::WaitFifoAdmissionBlock {
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

fn resource_lane_key(key: &str) -> store::ResourceLaneKey {
    store::ResourceLaneKey {
        namespace: ResourceNamespace::new("mfm.test.account_nonce").expect("namespace"),
        key_schema_id: schema_id(0x7b),
        key: events::ResourceKey::new(key).expect("resource key"),
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
