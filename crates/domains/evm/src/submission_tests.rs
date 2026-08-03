use std::collections::BTreeMap;

use alloy_primitives::{Address, B256, U256};
use mfm_canonical::{CanonicalValue, RecoverabilityContract};
use mfm_ids::{InvocationIdentity, RunId, StableId, StoreScopeId, TenantScopeId};
use mfm_program::structured::{
    CapabilityExpansion, CommittedObservation, RuntimeEffectCapability, RuntimeSigner, State,
    StateSettlement,
};
use mfm_signing::{
    PublicKeyBytes, PublicSigningIdentity, SigningAlgorithmId,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID,
};
use mfm_spec::structured::{
    AuthoredBlock, AuthoredDeclaration, AuthoredFailureDirective, AuthoredStructuredProgram,
    BlockTail, ProposedStateOutcome, ProposedStateValue,
};
use serde::de::DeserializeOwned;

use crate::submission::{
    ActiveCandidateWork, BroadcastExactCandidateState, CandidateActivationDecision,
    CandidateObservationWork, CandidateResolution, CandidateSlotDecision, CandidateWork,
    CompletedProjection, CompletionWork, DerivedSubmissionDomain, FailureReconciliationRequest,
    IntentBoundSubmission, ObservationRoundDecision, ObserveActivatedTransactionState,
    ObserveCandidateReceiptState, PendingEvmSubmissionFailure, PermittedCandidateWork,
    PostReservePreparedSubmission, PreparedCandidateActivation, PreparedWalletSubmission,
    QualifiedPendingSubmission, ReadCandidateStatusAfterFailureState,
    ReadReservationStatusAfterFailureState, SelectCandidateSlotState, SubmissionProgress,
    SubmissionWork, TerminalEvidenceDecision, TerminalEvidenceWork, WalletStatusBaseline,
    WalletStatusDecision,
};
use crate::submission_expansion::{
    candidate_attempt_recipe, evm_submission_recipe_with_candidate_slots,
    initial_reservation_recipe, PendingEvmSubmissionFailureRoute,
};
use crate::submission_process;
use crate::submission_registry::QualificationFixture;
use crate::{
    canonical_wallet_reference, derive_evm_semantic_signer_id,
    evm_deterministic_signing_profile_ref, evm_wallet_assurance_policy_ref,
    BroadcastExactCandidateCapability, ChainInstanceDeclaration, ChainInstanceRegistryAttestation,
    CompleteEvmNonceRequest, CompletedWalletNonce, EvmCallerSubmissionToken, EvmCandidateFamily,
    EvmCandidateSigner, EvmNetworkBinding, EvmRoutingCatalogDescriptor,
    EvmRoutingGenerationDescriptor, EvmSubmissionConfiguration, EvmSubmissionExpansion,
    EvmSubmissionFailure, EvmSubmissionRequest, EvmTransactionIntent, EvmWalletFeeCandidate,
    EvmWalletReference, TerminalWitnesses, WalletAuthorityContractError,
    WalletNonceDomainActivationAttestation, WalletNonceDomainActivationRecord, WalletNonceStatus,
    EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION, EVM_WALLET_REPLACEMENT_LIMIT,
};

#[test]
fn candidate_families_require_strict_increase_on_both_fee_axes() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let intent = fixture.prepared.intent.derived.request.transaction_intent();
    let candidate = |maximum: u64, priority: u64| {
        EvmWalletFeeCandidate::new(U256::from(maximum), U256::from(priority))
            .expect("valid individual fee candidate")
    };

    for candidates in [
        vec![candidate(20, 2), candidate(20, 3)],
        vec![candidate(20, 2), candidate(19, 3)],
        vec![candidate(20, 2), candidate(30, 2)],
        vec![candidate(20, 2), candidate(30, 1)],
    ] {
        assert_eq!(
            EvmCandidateFamily::new(intent, candidates),
            Err(WalletAuthorityContractError::Invalid(
                "candidate_fee_progression"
            ))
        );
    }

    let valid_family = EvmCandidateFamily::new(intent, vec![candidate(20, 2), candidate(30, 3)])
        .expect("strictly increasing candidate family");
    let mut wire = serde_json::to_value(&valid_family).expect("serialize candidate family");
    wire["candidates"][1]["max_priority_fee_per_gas"] = serde_json::json!("2");
    assert!(serde_json::from_value::<EvmCandidateFamily>(wire).is_err());
}

#[test]
fn decimal_strings_reject_noncanonical_spellings() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let request = &fixture.prepared.intent.derived.request;
    let declaration = request
        .domain_activation_attestation()
        .current_schema_record
        .chain_instance_attestation
        .declaration();
    let activation = &request
        .domain_activation_attestation()
        .current_schema_record;

    for invalid in ["00", "01", "+1", " 1", "1 "] {
        let mut declaration_wire =
            serde_json::to_value(declaration).expect("serialize chain declaration");
        declaration_wire["finalized_block_number"] = serde_json::json!(invalid);
        let decoded: ChainInstanceDeclaration =
            serde_json::from_value(declaration_wire).expect("decode structural declaration");
        assert!(
            decoded.validate().is_err(),
            "accepted declaration {invalid:?}"
        );

        let mut activation_wire =
            serde_json::to_value(activation).expect("serialize activation record");
        activation_wire["finalized_block_number"] = serde_json::json!(invalid);
        let decoded: WalletNonceDomainActivationRecord =
            serde_json::from_value(activation_wire).expect("decode structural activation");
        assert!(
            decoded.validate().is_err(),
            "accepted activation {invalid:?}"
        );

        assert!(
            !crate::submission::validate_quantity(invalid),
            "accepted submission quantity {invalid:?}"
        );
    }
}

#[test]
fn submission_rejects_a_foreign_qualified_chain_identity() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let request = &fixture.prepared.intent.derived.request;
    let activation = &request
        .domain_activation_attestation()
        .current_schema_record;
    let chain_attestation = &activation.chain_instance_attestation;
    let foreign_declaration = ChainInstanceDeclaration::new(
        chain_attestation
            .declaration()
            .chain_registry_lineage_ref()
            .clone(),
        StableId::new("mfm.evm.fixture/foreign-chain-instance").expect("foreign namespace"),
        request.transaction_intent().chain_id(),
        B256::repeat_byte(0x11),
        U256::from(10_u64),
        B256::repeat_byte(0x13),
    )
    .expect("foreign chain declaration");
    let foreign_attestation = ChainInstanceRegistryAttestation::new(
        foreign_declaration,
        chain_attestation.registry_issuance_ref().clone(),
        chain_attestation.registry_head_ref_at_issuance().clone(),
    )
    .expect("foreign chain attestation");
    let foreign_intent = EvmTransactionIntent::new(
        foreign_attestation
            .binding()
            .expect("foreign chain binding"),
        request.transaction_intent().nonce_domain().clone(),
        request.transaction_intent().template().clone(),
        StableId::new(request.transaction_intent().semantic_signer_id()).expect("semantic signer"),
        request
            .transaction_intent()
            .signing_profile_contract_ref()
            .clone(),
        request
            .transaction_intent()
            .submission_contract_ref()
            .clone(),
        request
            .transaction_intent()
            .terminal_assurance_contract_ref()
            .clone(),
    )
    .expect("self-consistent foreign intent");
    let foreign_family = EvmCandidateFamily::new(
        &foreign_intent,
        request.candidate_family().candidates().to_vec(),
    )
    .expect("foreign candidate family");

    assert_eq!(
        EvmSubmissionConfiguration::new(
            request.domain_activation_attestation().clone(),
            request.issuer_namespace_contract_ref().clone(),
            request.route_generation_ref().clone(),
            foreign_intent,
            foreign_family,
            request.observation_rounds(),
        ),
        Err(WalletAuthorityContractError::Invalid("submission_domain"))
    );
}

#[test]
fn configured_submission_cannot_assert_authenticated_identity() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let configuration = configuration_from_request(&fixture.prepared.intent.derived.request);
    let wire = serde_json::to_value(&configuration).expect("serialize configuration");
    let object = wire.as_object().expect("configuration object");

    for forbidden in [
        "tenant_scope_id",
        "authenticated_principal_id",
        "caller_submission_token",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "configured submission retained self-asserted {forbidden}"
        );
    }
    assert_value_schema(&configuration, "submission configuration");
}

#[test]
fn deployment_semantics_reject_syntax_valid_substitutes_and_derive_known_contracts() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let request = &fixture.prepared.intent.derived.request;
    let identity = |address: Address, public_key_byte: u8| {
        PublicSigningIdentity::new(
            SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
                .expect("algorithm"),
            Some(PublicKeyBytes::new(vec![public_key_byte; 33]).expect("public key")),
            Some(format!("{address:#x}")),
        )
        .expect("public identity")
    };
    let first_identity = identity(Address::repeat_byte(0x21), 0x02);
    let second_identity = identity(Address::repeat_byte(0x22), 0x03);
    let first_signer_id =
        derive_evm_semantic_signer_id(&first_identity).expect("first semantic signer");
    let repeated_first_signer_id =
        derive_evm_semantic_signer_id(&first_identity).expect("repeated semantic signer");
    let second_signer_id =
        derive_evm_semantic_signer_id(&second_identity).expect("second semantic signer");
    assert_eq!(first_signer_id, repeated_first_signer_id);
    assert_ne!(first_signer_id, second_signer_id);
    assert_eq!(
        configuration_from_request(request).deployment_semantics(&first_signer_id, &first_identity),
        Err(WalletAuthorityContractError::Invalid(
            "deployment_semantics"
        ))
    );

    let signer_contract = EvmCandidateSigner::contract().expect("candidate signer contract");
    let broadcast_contract = BroadcastExactCandidateCapability::contract()
        .and_then(|contract| contract.content_ref().map_err(Into::into))
        .expect("broadcast contract");
    let intent = EvmTransactionIntent::new(
        request.transaction_intent().chain_instance().clone(),
        request.transaction_intent().nonce_domain().clone(),
        request.transaction_intent().template().clone(),
        first_signer_id.clone(),
        evm_deterministic_signing_profile_ref().expect("signing profile"),
        EvmWalletReference::from_content_ref(broadcast_contract.clone()),
        evm_wallet_assurance_policy_ref().expect("assurance policy"),
    )
    .expect("canonical intent");
    let family = EvmCandidateFamily::new(&intent, request.candidate_family().candidates().to_vec())
        .expect("canonical family");
    let configuration = EvmSubmissionConfiguration::new(
        request.domain_activation_attestation().clone(),
        request.issuer_namespace_contract_ref().clone(),
        request.route_generation_ref().clone(),
        intent,
        family,
        request.observation_rounds(),
    )
    .expect("canonical configuration");
    let semantics = configuration
        .deployment_semantics(&first_signer_id, &first_identity)
        .expect("known deployment semantics");
    assert_eq!(semantics.semantic_signer_id(), &first_signer_id);
    assert_eq!(
        semantics.semantic_signer_contract_ref(),
        &EvmWalletReference::from_content_ref(
            signer_contract
                .content_ref()
                .expect("candidate signer contract ref")
        )
    );
    assert_eq!(
        configuration.deployment_semantics(&second_signer_id, &second_identity),
        Err(WalletAuthorityContractError::Invalid(
            "deployment_semantics"
        ))
    );
    let mismatched_sender_intent = EvmTransactionIntent::new(
        request.transaction_intent().chain_instance().clone(),
        request.transaction_intent().nonce_domain().clone(),
        request.transaction_intent().template().clone(),
        second_signer_id.clone(),
        evm_deterministic_signing_profile_ref().expect("signing profile"),
        EvmWalletReference::from_content_ref(broadcast_contract.clone()),
        evm_wallet_assurance_policy_ref().expect("assurance policy"),
    )
    .expect("different-key intent");
    let mismatched_sender_family = EvmCandidateFamily::new(
        &mismatched_sender_intent,
        request.candidate_family().candidates().to_vec(),
    )
    .expect("different-key family");
    let mismatched_sender_configuration = EvmSubmissionConfiguration::new(
        request.domain_activation_attestation().clone(),
        request.issuer_namespace_contract_ref().clone(),
        request.route_generation_ref().clone(),
        mismatched_sender_intent,
        mismatched_sender_family,
        request.observation_rounds(),
    )
    .expect("different-key configuration");
    assert_eq!(
        mismatched_sender_configuration.deployment_semantics(&second_signer_id, &second_identity),
        Err(WalletAuthorityContractError::Invalid(
            "deployment_semantics"
        ))
    );
    assert_eq!(
        semantics.broadcast_contract_ref(),
        &EvmWalletReference::from_content_ref(broadcast_contract)
    );
    assert_eq!(
        semantics.terminal_assurance_contract_ref(),
        &evm_wallet_assurance_policy_ref().expect("assurance policy")
    );
    assert_eq!(
        semantics.expected_sender().to_string().to_lowercase(),
        request.transaction_intent().nonce_domain().sender()
    );
}

#[test]
fn the_same_caller_token_shares_permanent_progress_across_distinct_runs() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let template = &fixture.prepared.intent.derived.request;
    let tenant = TenantScopeId::new(template.tenant_scope_id()).expect("tenant");
    let principal = StableId::new(template.authenticated_principal_id()).expect("principal");
    let token = EvmCallerSubmissionToken::new("retry-across-runs").expect("caller token");
    let first_request = EvmSubmissionRequest::from_authorized(
        configuration_from_request(template),
        tenant.clone(),
        principal.clone(),
        token.clone(),
    )
    .expect("first authorized request");
    let second_request = EvmSubmissionRequest::from_authorized(
        configuration_from_request(template),
        tenant.clone(),
        principal,
        token,
    )
    .expect("second authorized request");

    let first_run = test_run_id(
        &tenant,
        InvocationIdentity::new("00000000-0000-4000-8000-000000000071").expect("first invocation"),
    );
    let second_run = test_run_id(
        &tenant,
        InvocationIdentity::new("00000000-0000-4000-8000-000000000072").expect("second invocation"),
    );
    let first_progress = permanent_progress(first_request);
    let second_progress = permanent_progress(second_request);

    assert_ne!(
        first_run, second_run,
        "invocation identity must scope run ids"
    );
    assert_eq!(
        first_progress, second_progress,
        "caller token must bind run-independent intent and reservation progress"
    );
}

#[test]
fn the_same_caller_token_is_scoped_by_authenticated_principal() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let template = &fixture.prepared.intent.derived.request;
    let tenant = TenantScopeId::new(template.tenant_scope_id()).expect("tenant");
    let first = EvmSubmissionRequest::from_authorized(
        configuration_from_request(template),
        tenant.clone(),
        StableId::new("mfm.evm.test/principal-a").expect("first principal"),
        EvmCallerSubmissionToken::new("principal-scoped-token").expect("caller token"),
    )
    .expect("first principal request");
    let second = EvmSubmissionRequest::from_authorized(
        configuration_from_request(template),
        tenant,
        StableId::new("mfm.evm.test/principal-b").expect("second principal"),
        EvmCallerSubmissionToken::new("principal-scoped-token").expect("caller token"),
    )
    .expect("second principal request");

    assert_ne!(
        permanent_progress(first),
        permanent_progress(second),
        "two authenticated principals must not share permanent progress"
    );
}

#[test]
fn caller_submission_tokens_are_bounded_before_authorization() {
    assert!(EvmCallerSubmissionToken::new("caller-token").is_ok());
    assert!(EvmCallerSubmissionToken::new("").is_err());
    assert!(EvmCallerSubmissionToken::new("contains whitespace").is_err());
    assert!(EvmCallerSubmissionToken::new("x".repeat(257)).is_err());
}

#[test]
fn qualification_fixture_values_match_their_complete_schema_shapes() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let request = &fixture.prepared.intent.derived.request;
    assert_value_schema(
        &configuration_from_request(request),
        "submission configuration",
    );
    assert_value_schema(request, "submission request");
    assert_value_schema(&fixture.prepared, "prepared submission");
    assert_value_schema(&fixture.post_reserve, "post-reserve submission");
    assert_value_schema(
        &fixture.reservation_reconciliation,
        "reservation reconciliation",
    );
    assert_value_schema(
        &fixture.candidate_reconciliation,
        "candidate reconciliation",
    );
    assert_value_schema(&fixture.qualified_pending, "qualified pending submission");
    assert_value_schema(&fixture.candidate, "candidate work");
    assert_value_schema(&fixture.prepared_activation, "prepared activation");
    assert_value_schema(&fixture.active, "active candidate work");
    assert_value_schema(&fixture.observation, "candidate observation");
    assert_value_schema(&fixture.terminal, "terminal evidence");
    assert_value_schema(&fixture.completion, "completion work");
}

#[test]
fn retired_chain_qualification_wire_shapes_are_rejected_without_compatibility_readers() {
    type Decoder = fn(serde_json::Value) -> Result<(), serde_json::Error>;

    let fixture = QualificationFixture::new().expect("qualification fixture");
    let request = &fixture.prepared.intent.derived.request;
    let activation = request.domain_activation_attestation();
    let activation_record = &activation.current_schema_record;
    let declaration = activation_record.chain_instance_attestation.declaration();
    let binding = activation_record
        .chain_instance_attestation
        .binding()
        .expect("chain binding");

    let mut retired_declaration = serde_json::to_value(declaration).expect("chain declaration");
    let declaration_object = retired_declaration
        .as_object_mut()
        .expect("chain declaration object");
    declaration_object.remove("qualified_chain_registry_lineage_ref");
    declaration_object.insert(
        "stable_chain_registry_id".to_owned(),
        serde_json::Value::String("mfm.evm.fixture/retired-registry".to_owned()),
    );

    let mut retired_activation_record =
        serde_json::to_value(activation_record).expect("activation record");
    let activation_object = retired_activation_record
        .as_object_mut()
        .expect("activation record object");
    activation_object.remove("chain_instance_attestation");
    activation_object.remove("initial_route_generation_ref");
    activation_object.remove("initial_route_membership_issuance_ref");
    activation_object.insert(
        "qualified_chain_instance_declaration_ref".to_owned(),
        serde_json::to_value(
            canonical_wallet_reference(declaration).expect("retired declaration reference"),
        )
        .expect("retired declaration reference JSON"),
    );

    let mut retired_activation = serde_json::to_value(activation).expect("activation attestation");
    retired_activation["current_schema_record"] = retired_activation_record.clone();

    let mut retired_intent =
        serde_json::to_value(request.transaction_intent()).expect("transaction intent");
    let intent_object = retired_intent
        .as_object_mut()
        .expect("transaction intent object");
    intent_object.remove("chain_instance");
    intent_object.insert(
        "qualified_chain_instance_id".to_owned(),
        serde_json::to_value(binding.qualified_chain_instance_id())
            .expect("qualified chain identity JSON"),
    );
    intent_object.insert(
        "chain_id".to_owned(),
        serde_json::Value::from(binding.chain_id()),
    );

    let mut retired_request = serde_json::to_value(request).expect("submission request");
    retired_request["domain_activation_attestation"] = retired_activation.clone();
    retired_request["transaction_intent"] = retired_intent.clone();
    retired_request["chain_declaration"] = retired_declaration.clone();

    let network_binding = EvmNetworkBinding::new(
        "retired-network",
        binding.clone(),
        activation_record.initial_route_generation_ref.clone(),
    )
    .expect("current network binding");
    let mut retired_network = serde_json::to_value(network_binding).expect("network binding");
    let network_object = retired_network
        .as_object_mut()
        .expect("network binding object");
    network_object.remove("chain_instance");
    network_object.insert(
        "chain_id".to_owned(),
        serde_json::Value::from(binding.chain_id()),
    );

    let route = EvmRoutingGenerationDescriptor::new(
        "retired-network",
        "retired-source",
        StableId::new("mfm.evm.fixture/retired-generation").expect("generation id"),
        activation_record
            .initial_route_membership_issuance_ref
            .clone(),
        binding,
    )
    .expect("current route descriptor");
    let mut retired_route = serde_json::to_value(route).expect("route descriptor");
    let route_object = retired_route
        .as_object_mut()
        .expect("route descriptor object");
    route_object.remove("chain_instance");
    route_object.remove("route_membership_issuance_ref");
    route_object.insert("chain_id".to_owned(), serde_json::Value::from(1_u64));

    let retired_catalog = serde_json::json!({
        "version": EVM_ROUTING_CATALOG_DESCRIPTOR_VERSION,
        "ordered_generation_refs": [
            serde_json::to_value(activation_record.initial_route_generation_ref.clone())
                .expect("retired generation reference JSON"),
        ],
    });

    let mut cases: Vec<(&str, serde_json::Value, Decoder)> = vec![
        (
            "chain declaration with caller-selected registry id",
            retired_declaration,
            decode_value::<ChainInstanceDeclaration>,
        ),
        (
            "activation record with declaration-only chain reference",
            retired_activation_record,
            decode_value::<WalletNonceDomainActivationRecord>,
        ),
        (
            "activation attestation containing the retired record",
            retired_activation,
            decode_value::<WalletNonceDomainActivationAttestation>,
        ),
        (
            "transaction intent with independent chain id fields",
            retired_intent,
            decode_value::<EvmTransactionIntent>,
        ),
        (
            "submission request with caller-supplied chain declaration",
            retired_request.clone(),
            decode_value::<EvmSubmissionRequest>,
        ),
        (
            "network binding with numeric chain id only",
            retired_network,
            decode_value::<EvmNetworkBinding>,
        ),
        (
            "route descriptor without provider membership issuance",
            retired_route,
            decode_value::<EvmRoutingGenerationDescriptor>,
        ),
        (
            "aggregate routing catalog with ordered generation refs only",
            retired_catalog,
            decode_value::<EvmRoutingCatalogDescriptor>,
        ),
    ];

    macro_rules! retired_intermediate {
        ($label:literal, $value:expr, [$($segment:literal),+], $type:ty) => {{
            let mut value = serde_json::to_value($value).expect($label);
            replace_json_path(&mut value, &[$($segment),+], retired_request.clone());
            cases.push(($label, value, decode_value::<$type>));
        }};
    }
    retired_intermediate!(
        "derived intermediate carrying a retired request",
        &fixture.prepared.intent.derived,
        ["request"],
        DerivedSubmissionDomain
    );
    retired_intermediate!(
        "intent-bound intermediate carrying a retired request",
        &fixture.prepared.intent,
        ["derived", "request"],
        IntentBoundSubmission
    );
    retired_intermediate!(
        "prepared intermediate carrying a retired request",
        &fixture.prepared,
        ["intent", "derived", "request"],
        PreparedWalletSubmission
    );
    retired_intermediate!(
        "post-reserve intermediate carrying a retired request",
        &fixture.post_reserve,
        ["prepared", "intent", "derived", "request"],
        PostReservePreparedSubmission
    );
    retired_intermediate!(
        "reservation reconciliation carrying a retired request",
        &fixture.reservation_reconciliation,
        ["prepared", "intent", "derived", "request"],
        FailureReconciliationRequest
    );
    retired_intermediate!(
        "qualified pending intermediate carrying a retired request",
        &fixture.qualified_pending,
        ["prepared", "intent", "derived", "request"],
        QualifiedPendingSubmission
    );
    retired_intermediate!(
        "submission work carrying a retired request",
        &fixture.candidate.work,
        ["prepared", "intent", "derived", "request"],
        SubmissionWork
    );
    retired_intermediate!(
        "candidate work carrying a retired request",
        &fixture.candidate,
        ["work", "prepared", "intent", "derived", "request"],
        CandidateWork
    );
    retired_intermediate!(
        "prepared activation carrying a retired request",
        &fixture.prepared_activation,
        [
            "candidate",
            "work",
            "prepared",
            "intent",
            "derived",
            "request"
        ],
        PreparedCandidateActivation
    );
    retired_intermediate!(
        "active candidate carrying a retired request",
        &fixture.active,
        [
            "candidate",
            "work",
            "prepared",
            "intent",
            "derived",
            "request"
        ],
        ActiveCandidateWork
    );
    retired_intermediate!(
        "candidate observation carrying a retired request",
        &fixture.observation,
        [
            "active",
            "candidate",
            "work",
            "prepared",
            "intent",
            "derived",
            "request"
        ],
        CandidateObservationWork
    );
    retired_intermediate!(
        "terminal evidence carrying a retired request",
        &fixture.terminal,
        [
            "candidate",
            "active",
            "candidate",
            "work",
            "prepared",
            "intent",
            "derived",
            "request"
        ],
        TerminalEvidenceWork
    );
    retired_intermediate!(
        "completion work carrying a retired request",
        &fixture.completion,
        [
            "submission_work",
            "prepared",
            "intent",
            "derived",
            "request"
        ],
        CompletionWork
    );

    for (label, bytes, decode) in cases {
        assert!(
            decode(bytes).is_err(),
            "retired {label} unexpectedly decoded"
        );
    }
}

fn decode_value<T: DeserializeOwned>(value: serde_json::Value) -> Result<(), serde_json::Error> {
    serde_json::from_value::<T>(value).map(|_| ())
}

fn configuration_from_request(request: &EvmSubmissionRequest) -> EvmSubmissionConfiguration {
    EvmSubmissionConfiguration::new(
        request.domain_activation_attestation().clone(),
        request.issuer_namespace_contract_ref().clone(),
        request.route_generation_ref().clone(),
        request.transaction_intent().clone(),
        request.candidate_family().clone(),
        request.observation_rounds(),
    )
    .expect("submission configuration")
}

fn permanent_progress(request: EvmSubmissionRequest) -> (String, String) {
    let derived = successful(submission_process::derive_transaction_intent(&request));
    let intent = successful(submission_process::derive_submission_intent(&derived));
    let prepared = successful(submission_process::derive_reservation_key(&intent));
    (
        prepared.intent.submission_intent_id.as_str().to_owned(),
        prepared.reservation_key.as_str().to_owned(),
    )
}

fn successful<T, E>(outcome: ProposedStateOutcome<T, E>) -> T {
    match outcome.into_parts().0 {
        ProposedStateValue::Success(value) => value,
        ProposedStateValue::Failure(_) => panic!("unexpected submission failure"),
    }
}

fn test_run_id(tenant: &TenantScopeId, invocation: InvocationIdentity) -> RunId {
    let store = StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, "7".repeat(32)))
        .expect("store scope");
    let operation = StableId::new("mfm.evm/submit-transaction").expect("operation");
    let preimage = CanonicalValue::object([
        (
            "store_scope_id",
            CanonicalValue::String(store.as_str().to_owned()),
        ),
        (
            "tenant_scope_id",
            CanonicalValue::String(tenant.as_str().to_owned()),
        ),
        (
            "entry_point_operation_id",
            CanonicalValue::String(operation.as_str().to_owned()),
        ),
        (
            "invocation_identity",
            CanonicalValue::String(invocation.as_str().to_owned()),
        ),
    ])
    .expect("run preimage");
    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    let encoded = contract
        .encode("mfm.run-id-preimage.v1", &preimage)
        .expect("validated run preimage");
    contract.derive_run_id(&encoded).expect("run id")
}

fn replace_json_path(value: &mut serde_json::Value, path: &[&str], replacement: serde_json::Value) {
    let (field, parents) = path.split_last().expect("non-empty JSON path");
    let mut current = value;
    for segment in parents {
        current = current
            .get_mut(*segment)
            .unwrap_or_else(|| panic!("missing JSON path segment {segment}"));
    }
    current[*field] = replacement;
}

fn assert_value_schema<T>(value: &T, label: &str)
where
    T: mfm_values::MfmValue + serde::Serialize,
{
    let canonical = mfm_canonical::PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(value).unwrap_or_else(|error| panic!("encode {label}: {error}")),
    )
    .unwrap_or_else(|error| panic!("canonicalize {label}: {error}"));
    let descriptor = T::schema_descriptor().unwrap_or_else(|error| panic!("{label}: {error}"));
    descriptor
        .identity()
        .validate_canonical_value(canonical.as_bytes())
        .unwrap_or_else(|error| panic!("serialized {label} differs from its schema: {error}"));
}

#[test]
fn completion_requests_reject_forged_terminal_witness_cross_links() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let request = fixture.completion.request.clone();
    assert!(request.validate().is_ok());

    let reject = |label: &str, forged: CompleteEvmNonceRequest| {
        assert!(forged.validate().is_err(), "accepted forged {label}");
    };
    let forged_hash = format!("{:#x}", B256::repeat_byte(0xa1));

    let mut forged = request.clone();
    if let crate::EvmTransactionLookupObservation::Found {
        transaction_hash, ..
    } = &mut forged.terminal_witnesses.transaction
    {
        *transaction_hash = forged_hash.clone();
    }
    reject("transaction hash", forged);

    let mut forged = request.clone();
    if let crate::EvmReceiptLookupObservation::Found {
        transaction_hash, ..
    } = &mut forged.terminal_witnesses.receipt
    {
        *transaction_hash = forged_hash;
    }
    reject("receipt hash", forged);

    let mut forged = request.clone();
    forged.terminal_witnesses.inclusion_block.block_number = "9".to_owned();
    reject("inclusion block", forged);

    let mut forged = request.clone();
    forged.terminal_witnesses.finalized_head.block_number = "9".to_owned();
    reject("finalized head", forged);

    let mut forged = request.clone();
    if let crate::EvmReceiptLookupObservation::Found { status, .. } =
        &mut forged.terminal_witnesses.receipt
    {
        *status = 0;
    }
    reject("execution status", forged);

    let mut forged = request.clone();
    forged.terminal_witnesses.terminal_assurance_contract_ref = forged
        .canonical_terminal_outcome
        .winning_activation_evidence_ref
        .clone();
    reject("terminal assurance", forged);

    let mut forged = request;
    forged.canonical_terminal_outcome.canonical_public_result = "{\"conflict\":true}".to_owned();
    forged.terminal_witnesses.canonical_public_result = "{\"conflict\":true}".to_owned();
    reject("canonical public projection", forged);
}

#[test]
fn submission_expansion_freezes_all_candidate_and_observation_occurrences() {
    assert_schema::<WalletNonceDomainActivationAttestation>("activation attestation");
    assert_schema::<EvmSubmissionConfiguration>("submission configuration");
    assert_schema::<EvmSubmissionRequest>("submission request");
    assert_schema::<DerivedSubmissionDomain>("derived submission");
    assert_schema::<IntentBoundSubmission>("intent-bound submission");
    assert_schema::<PreparedWalletSubmission>("prepared submission");
    assert_schema::<WalletStatusBaseline>("wallet status baseline");
    assert_schema::<FailureReconciliationRequest>("failure reconciliation request");
    assert_schema::<PendingEvmSubmissionFailure>("pending submission failure");
    assert_schema::<PendingEvmSubmissionFailureRoute>("pending submission failure route");
    assert_schema::<PostReservePreparedSubmission>("post-reserve prepared submission");
    assert_schema::<SubmissionWork>("submission work");
    assert_schema::<WalletNonceStatus>("wallet nonce status");
    assert_schema::<WalletStatusDecision>("wallet status decision");
    assert_schema::<QualifiedPendingSubmission>("qualified pending submission");
    assert_schema::<SubmissionProgress>("submission progress");
    assert_schema::<CandidateSlotDecision>("candidate slot decision");
    assert_schema::<CandidateWork>("candidate work");
    assert_schema::<PermittedCandidateWork>("permitted candidate work");
    assert_schema::<PreparedCandidateActivation>("prepared candidate activation");
    assert_schema::<ActiveCandidateWork>("active candidate work");
    assert_schema::<CandidateActivationDecision>("candidate activation decision");
    assert_schema::<CandidateResolution>("candidate resolution");
    assert_schema::<CandidateObservationWork>("candidate observation work");
    assert_schema::<ObservationRoundDecision>("observation round decision");
    assert_schema::<TerminalEvidenceDecision>("terminal evidence decision");
    assert_schema::<TerminalEvidenceWork>("terminal evidence work");
    assert_schema::<TerminalWitnesses>("terminal witnesses");
    assert_schema::<CompleteEvmNonceRequest>("completion request");
    assert_schema::<CompletionWork>("completion work");
    assert_schema::<CompletedProjection>("completed projection");

    let recipe = EvmSubmissionExpansion::recipe().expect("submission expansion recipe");
    let repeated = EvmSubmissionExpansion::recipe().expect("repeated submission expansion recipe");
    assert_eq!(recipe, repeated);
    assert_eq!(
        recipe.content_ref().expect("submission recipe ref"),
        repeated
            .content_ref()
            .expect("repeated submission recipe ref")
    );
    let initial_child = initial_reservation_recipe().expect("initial reservation child");
    let candidate_child = candidate_attempt_recipe().expect("candidate attempt child");
    let initial_child_ref = initial_child.content_ref().expect("initial child ref");
    let candidate_child_ref = candidate_child.content_ref().expect("candidate child ref");
    let mut counts = BTreeMap::<String, usize>::new();
    count_states(&recipe.root, &mut counts);

    assert_eq!(
        counts.get(
            SelectCandidateSlotState::semantic_state_id()
                .expect("state id")
                .as_str()
        ),
        Some(&EVM_WALLET_REPLACEMENT_LIMIT)
    );
    assert_eq!(
        counts.get(
            ReadReservationStatusAfterFailureState::semantic_state_id()
                .expect("state id")
                .as_str()
        ),
        Some(&1)
    );
    assert_eq!(
        counts.get(
            ReadCandidateStatusAfterFailureState::semantic_state_id()
                .expect("state id")
                .as_str()
        ),
        Some(&EVM_WALLET_REPLACEMENT_LIMIT)
    );
    assert_eq!(count_child_calls(&recipe.root, &initial_child_ref), 1);
    assert_eq!(
        count_child_calls(&recipe.root, &candidate_child_ref),
        EVM_WALLET_REPLACEMENT_LIMIT
    );

    let mut candidate_counts = BTreeMap::<String, usize>::new();
    count_states(&candidate_child.root, &mut candidate_counts);
    assert_eq!(
        candidate_counts.get(
            BroadcastExactCandidateState::semantic_state_id()
                .expect("state id")
                .as_str()
        ),
        Some(&1)
    );
    assert_eq!(
        candidate_counts.get(
            ObserveActivatedTransactionState::semantic_state_id()
                .expect("state id")
                .as_str()
        ),
        Some(&2)
    );
    assert_eq!(
        candidate_counts.get(
            ObserveCandidateReceiptState::semantic_state_id()
                .expect("state id")
                .as_str()
        ),
        Some(&2)
    );
    assert_eq!(count_all_child_calls(&initial_child.root), 0);
    assert_eq!(count_all_child_calls(&candidate_child.root), 0);
    assert!(recipe.content_ref().is_ok());
}

#[test]
fn candidate_skip_provenance_is_normalized_and_preserves_exact_progress() {
    let one_slot = evm_submission_recipe_with_candidate_slots(1).expect("one candidate slot");
    let all_slots = evm_submission_recipe_with_candidate_slots(
        u16::try_from(EVM_WALLET_REPLACEMENT_LIMIT).expect("replacement limit fits u16"),
    )
    .expect("all candidate slots");
    let one_slot_bytes = one_slot.canonical_json().expect("one-slot canonical bytes");
    let all_slot_bytes = all_slots
        .canonical_json()
        .expect("all-slot canonical bytes");
    assert!(all_slot_bytes.as_bytes().len() > one_slot_bytes.as_bytes().len());
    assert!(all_slot_bytes.as_bytes().len() < 16_777_216);
    let serialized: serde_json::Value =
        serde_json::from_slice(all_slot_bytes.as_bytes()).expect("normalized authored recipe JSON");
    assert!(serialized["structural_paths"].is_array());
    assert!(serialized["lexical_slots"].is_array());
    assert!(!contains_inline_lexical_slot(&serialized["root"]));
    assert!(serialized["lexical_slots"]
        .as_array()
        .expect("lexical slot table")
        .iter()
        .all(|definition| serde_json::to_vec(&definition["slot"])
            .expect("shallow slot bytes")
            .len()
            < 8_192));
    let decoded: AuthoredStructuredProgram = serde_json::from_slice(all_slot_bytes.as_bytes())
        .expect("strict normalized authored recipe roundtrip");
    assert_eq!(decoded, all_slots);
    assert_eq!(
        count_direct_candidate_skip_arms(&all_slots.root),
        EVM_WALLET_REPLACEMENT_LIMIT
    );

    let fixture = QualificationFixture::new().expect("qualification fixture");
    let completed = match completed_status(&fixture) {
        WalletNonceStatus::Completed { completion, .. } => SubmissionProgress {
            work: None,
            completion: Some(completion),
            failure: None,
        },
        _ => unreachable!("completed fixture"),
    };
    let failed = SubmissionProgress {
        work: None,
        completion: None,
        failure: Some(EvmSubmissionFailure::ProviderUnavailable),
    };
    for progress in [completed, failed] {
        assert_eq!(
            submission_process::select_candidate_slot(&progress),
            ProposedStateOutcome::Success(CandidateSlotDecision::Skip)
        );
    }
}

#[test]
fn recovery_visits_every_activated_candidate_before_replacement() {
    use crate::{derive_exact_candidate_activation_permit, CandidateActivationPermit};

    let fixture = QualificationFixture::new().expect("qualification fixture");
    // Crash mid-history with ≥2 activated candidates retained.
    let c0 = fixture.active.active_candidate.clone();
    let mut c1 = c0.clone();
    c1.attested_candidate.candidate_ordinal = 1;
    c1.attested_candidate.transaction_hash = format!("{:#x}", B256::repeat_byte(0x42));
    c1.attested_candidate.unsigned_candidate_digest = format!("{:#x}", B256::repeat_byte(0x62));
    // Distinct descriptor identity so prefix validation elsewhere stays strict.
    c1.attested_candidate.candidate_descriptor_ref = fixture
        .prepared
        .intent
        .derived
        .request
        .route_generation_ref()
        .clone();
    c1.activation_evidence_ref = fixture
        .post_reserve
        .reservation
        .reservation_evidence_ref
        .clone();

    let mut work = fixture.candidate.work.clone();
    work.activated_candidates = vec![c0.clone(), c1.clone()];
    work.current_candidate = Some(c1.clone());
    // Simulate status recovery: walk starts at ordinal 0, not activated_len.
    work.next_candidate_ordinal = 0;
    work.observed_prefix_len = 0;

    let progress = SubmissionProgress {
        work: Some(work.clone()),
        completion: None,
        failure: None,
    };
    // Fixture family has one member; with next=0 still Execute for reobservation.
    let CandidateSlotDecision::Execute { work: slot0 } =
        successful(submission_process::select_candidate_slot(&progress))
    else {
        panic!("slot 0 must execute recovery of first activated candidate");
    };
    assert_eq!(slot0.next_candidate_ordinal, 0);

    let permit0 = derive_exact_candidate_activation_permit(
        &slot0.reservation,
        &slot0.activated_candidates,
        0,
        0,
    )
    .expect("reobservation of ordinal 0");
    assert!(matches!(
        permit0,
        CandidateActivationPermit::Reobservation {
            exact_ordinal: 0,
            ..
        }
    ));

    // EVM-04: replacement without observing the full activated prefix is rejected.
    assert!(
        derive_exact_candidate_activation_permit(
            &slot0.reservation,
            &slot0.activated_candidates,
            2,
            0,
        )
        .is_err(),
        "replacement without observing activated prefix must fail"
    );
    assert!(
        derive_exact_candidate_activation_permit(
            &slot0.reservation,
            &slot0.activated_candidates,
            2,
            1,
        )
        .is_err(),
        "replacement after partial observation must fail"
    );
    let permit_replace = derive_exact_candidate_activation_permit(
        &slot0.reservation,
        &slot0.activated_candidates,
        2,
        2,
    )
    .expect("replacement after independent observation of every earlier candidate");
    assert!(matches!(
        permit_replace,
        CandidateActivationPermit::Replacement {
            exact_next_ordinal: 2,
            predecessor_ordinal: 1,
            ..
        }
    ));

    // Observation reconcile advances past ordinal 0 without jumping to activated_len.
    let active0 = ActiveCandidateWork {
        candidate: CandidateWork {
            work: work.clone(),
            unsigned_candidate: fixture.candidate.unsigned_candidate.clone(),
            attested_candidate: Some(c0.attested_candidate.clone()),
        },
        active_candidate: c0.clone(),
    };
    let mut observation = fixture.observation.clone();
    observation.active = active0;
    observation.observation.transaction = Some(crate::EvmTransactionLookupObservation::Missing);
    observation.observation.receipt = Some(crate::EvmReceiptLookupObservation::Missing);
    let CandidateResolution::Resume { work: advanced } =
        successful(submission_process::mark_observation_reconcile(&observation))
    else {
        panic!("non-terminal observation must resume next ordinal");
    };
    assert_eq!(advanced.next_candidate_ordinal, 1);
    assert_eq!(advanced.observed_prefix_len, 1);
    assert_eq!(advanced.activated_candidates.len(), 2);

    // After observing c0, reobservation of c1 is the next certified step.
    let permit1 = derive_exact_candidate_activation_permit(
        &advanced.reservation,
        &advanced.activated_candidates,
        1,
        1,
    )
    .expect("reobservation of ordinal 1 after observing ordinal 0");
    assert!(matches!(
        permit1,
        CandidateActivationPermit::Reobservation {
            exact_ordinal: 1,
            ..
        }
    ));

    // Status resume still starts at 0 even when the retained prefix is non-empty.
    let multi_status = reserved_status(&fixture, true);
    // reserved_status(advanced=true) has one candidate; assert next resets to 0.
    let StateSettlement::Proposed(outcome) = submission_process::settle_wallet_status(
        &fixture.prepared,
        &CommittedObservation::Returned(multi_status),
    ) else {
        panic!("reserved status must settle");
    };
    let ProposedStateValue::Success(WalletStatusDecision::Reserved { work: resumed }) =
        outcome.value()
    else {
        panic!("must resume reserved work");
    };
    assert_eq!(resumed.next_candidate_ordinal, 0);
    assert_eq!(resumed.observed_prefix_len, 0);
    assert!(!resumed.activated_candidates.is_empty());
}

#[test]
fn completed_wallet_nonce_retains_rehashable_public_recovery_closure() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let completed = match completed_status(&fixture) {
        WalletNonceStatus::Completed { completion, .. } => completion,
        _ => panic!("completed fixture"),
    };
    completed
        .validate()
        .expect("complete public recovery closure must validate");
    // Output embeds the full witness preimage, not a dangling digest.
    assert_eq!(
        completed.terminal_witnesses,
        fixture.completion.request.terminal_witnesses
    );
    let witnesses_ref = canonical_wallet_reference(&completed.terminal_witnesses)
        .expect("witnesses reference");
    assert_eq!(
        completed.original_terminal_witnesses_ref,
        witnesses_ref.content_digest()
    );
    assert!(
        !completed.sealed_activated_candidates.is_empty(),
        "sealed activated prefix must be retained"
    );
    assert!(completed.sealed_activated_candidates.iter().any(|c| {
        c.attested_candidate.candidate_ordinal
            == completed
                .canonical_terminal_outcome
                .winning_candidate_ordinal
            && c.attested_candidate.transaction_hash
                == completed.canonical_terminal_outcome.transaction_hash
    }));

    let mut forged = completed.clone();
    forged.sealed_activated_candidates.clear();
    assert!(forged.validate().is_err(), "empty sealed prefix rejected");

    let mut forged = completed.clone();
    forged.original_terminal_witnesses_ref =
        format!("{:#x}", B256::repeat_byte(0xee));
    assert!(
        forged.validate().is_err(),
        "witness digest mismatch rejected"
    );

    let mut forged = completed;
    forged.terminal_witnesses.canonical_public_result = "{\"conflict\":true}".to_owned();
    assert!(
        forged.validate().is_err(),
        "witness/outcome conflict rejected"
    );
}

#[test]
fn failure_reconciliation_uses_one_exact_authoritative_snapshot() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    let unchanged = reserved_status(&fixture, false);
    assert_eq!(
        submission_process::settle_candidate_failure_status(
            &fixture.candidate_reconciliation,
            &CommittedObservation::Returned(unchanged),
        ),
        StateSettlement::Proposed(ProposedStateOutcome::Failure(
            EvmSubmissionFailure::ProviderUnavailable,
        ))
    );

    let changed = reserved_status(&fixture, true);
    let changed_digest = canonical_wallet_reference(&changed)
        .expect("changed status reference")
        .content_digest()
        .to_owned();
    let changed_head = match &changed {
        WalletNonceStatus::Reserved {
            resource_head_ref, ..
        } => resource_head_ref.clone(),
        _ => panic!("reserved fixture"),
    };
    let StateSettlement::Proposed(changed_resolution) =
        submission_process::settle_candidate_failure_status(
            &fixture.candidate_reconciliation,
            &CommittedObservation::Returned(changed),
        )
    else {
        panic!("changed status must settle");
    };
    let mfm_spec::structured::ProposedStateValue::Success(CandidateResolution::Resume { work }) =
        changed_resolution.value()
    else {
        panic!("changed status must resume");
    };
    assert_eq!(
        work.activated_candidates,
        vec![fixture.active.active_candidate.clone()]
    );
    assert_eq!(
        work.status_baseline,
        WalletStatusBaseline::Reserved {
            resource_head_ref: changed_head,
            canonical_status_digest: changed_digest,
        }
    );

    let completed = completed_status(&fixture);
    let expected_completion = match &completed {
        WalletNonceStatus::Completed { completion, .. } => completion.clone(),
        _ => panic!("completed fixture"),
    };
    assert_eq!(
        submission_process::settle_candidate_failure_status(
            &fixture.candidate_reconciliation,
            &CommittedObservation::Returned(completed),
        ),
        StateSettlement::Proposed(ProposedStateOutcome::Success(
            CandidateResolution::Completed {
                completion: expected_completion,
            },
        ))
    );
}

#[test]
fn reconciliation_status_reads_fail_closed_without_recursing() {
    let fixture = QualificationFixture::new().expect("qualification fixture");
    assert_eq!(
        submission_process::settle_candidate_failure_status(
            &fixture.reservation_reconciliation,
            &CommittedObservation::Returned(reserved_status(&fixture, false)),
        ),
        StateSettlement::InvalidEvidence
    );
    assert_eq!(
        submission_process::settle_candidate_failure_status(
            &fixture.candidate_reconciliation,
            &CommittedObservation::SafeFailure(EvmSubmissionFailure::NonceAuthorityUnavailable,),
        ),
        StateSettlement::Proposed(ProposedStateOutcome::Failure(
            EvmSubmissionFailure::NonceAuthorityUnavailable,
        ))
    );
    assert_eq!(
        submission_process::settle_candidate_wallet_status(
            &fixture.prepared,
            &CommittedObservation::SafeFailure(EvmSubmissionFailure::NonceAuthorityUnavailable,),
        ),
        StateSettlement::Proposed(ProposedStateOutcome::Failure(
            PendingEvmSubmissionFailure::Direct {
                failure: EvmSubmissionFailure::NonceAuthorityUnavailable,
            },
        ))
    );
    assert_eq!(
        submission_process::settle_post_reserve_wallet_status(
            &fixture.post_reserve,
            &CommittedObservation::Returned(WalletNonceStatus::Absent),
        ),
        StateSettlement::InvalidEvidence
    );
    assert_eq!(
        submission_process::settle_reservation_failure_status(
            &fixture.reservation_reconciliation,
            &CommittedObservation::Returned(WalletNonceStatus::Busy),
        ),
        StateSettlement::Proposed(ProposedStateOutcome::Failure(
            EvmSubmissionFailure::NonceDomainBusy,
        ))
    );
}

fn reserved_status(fixture: &QualificationFixture, advanced: bool) -> WalletNonceStatus {
    let request = &fixture.prepared.intent.derived.request;
    let (activated_candidates, current_candidate, resource_head_ref) = if advanced {
        let active = fixture.active.active_candidate.clone();
        let head = canonical_wallet_reference(&active).expect("advanced head");
        (vec![active.clone()], Some(active), head)
    } else {
        let WalletStatusBaseline::Reserved {
            resource_head_ref, ..
        } = &fixture.candidate_reconciliation.baseline
        else {
            panic!("candidate baseline");
        };
        (Vec::new(), None, resource_head_ref.clone())
    };
    WalletNonceStatus::Reserved {
        reservation: fixture.post_reserve.reservation.clone(),
        transaction_intent: request.transaction_intent().clone(),
        candidate_family: request.candidate_family().clone(),
        activated_candidates,
        current_candidate,
        resource_head_ref,
    }
}

fn completed_status(fixture: &QualificationFixture) -> WalletNonceStatus {
    let request = &fixture.prepared.intent.derived.request;
    let completion_request = &fixture.completion.request;
    let completion = CompletedWalletNonce {
        nonce_domain: completion_request.nonce_domain.clone(),
        nonce: completion_request.current_reservation.nonce,
        semantic_reservation_key: completion_request
            .current_reservation
            .semantic_reservation_key
            .clone(),
        semantic_completion_key: completion_request.completion_key.clone(),
        canonical_terminal_outcome: completion_request.canonical_terminal_outcome.clone(),
        terminal_witnesses: completion_request.terminal_witnesses.clone(),
        sealed_activated_candidates: vec![fixture.active.active_candidate.clone()],
        original_terminal_witnesses_ref: canonical_wallet_reference(
            &completion_request.terminal_witnesses,
        )
        .expect("terminal witness reference")
        .content_digest()
        .to_owned(),
        completion_evidence_ref: fixture
            .post_reserve
            .reservation
            .reservation_evidence_ref
            .clone(),
    };
    WalletNonceStatus::Completed {
        reservation: fixture.post_reserve.reservation.clone(),
        transaction_intent: request.transaction_intent().clone(),
        candidate_family: request.candidate_family().clone(),
        activated_candidates: vec![fixture.active.active_candidate.clone()],
        completion,
        resource_head_ref: canonical_wallet_reference(&fixture.active.active_candidate)
            .expect("completed head"),
    }
}

fn assert_schema<T: mfm_values::MfmValue>(label: &str) {
    let descriptor = T::schema_descriptor().unwrap_or_else(|error| panic!("{label}: {error}"));
    let identity = descriptor
        .identity_canonical_json()
        .unwrap_or_else(|error| panic!("{label}: {error}"));
    assert!(
        identity.as_bytes().len() <= 65_536,
        "{label} exceeds the canonical schema bound"
    );
}

fn count_states(block: &AuthoredBlock, counts: &mut BTreeMap<String, usize>) {
    for declaration in &block.declarations {
        match declaration {
            AuthoredDeclaration::State(state) => {
                *counts
                    .entry(state.contract.semantic_state_id.as_str().to_owned())
                    .or_default() += 1;
                count_failure_states(&state.failure_directive, counts);
            }
            AuthoredDeclaration::Match(binding) => {
                for arm in &binding.arms {
                    count_states(&arm.body, counts);
                }
            }
            AuthoredDeclaration::FanOut(group) => {
                for lane in &group.lanes {
                    count_states(&lane.body, counts);
                }
            }
            AuthoredDeclaration::OperationCall(call) => {
                count_failure_states(&call.failure_directive, counts);
            }
        }
    }
}

fn count_failure_states(
    directive: &AuthoredFailureDirective,
    counts: &mut BTreeMap<String, usize>,
) {
    if let AuthoredFailureDirective::Custom { arms, .. } = directive {
        for arm in arms {
            count_states(&arm.body, counts);
        }
    }
}

fn count_child_calls(block: &AuthoredBlock, target: &mfm_ids::ContentRef) -> usize {
    count_calls(block, Some(target))
}

fn count_all_child_calls(block: &AuthoredBlock) -> usize {
    count_calls(block, None)
}

fn count_calls(block: &AuthoredBlock, target: Option<&mfm_ids::ContentRef>) -> usize {
    block
        .declarations
        .iter()
        .map(|declaration| match declaration {
            AuthoredDeclaration::State(state) => {
                count_failure_calls(&state.failure_directive, target)
            }
            AuthoredDeclaration::Match(binding) => binding
                .arms
                .iter()
                .map(|arm| count_calls(&arm.body, target))
                .sum(),
            AuthoredDeclaration::FanOut(group) => group
                .lanes
                .iter()
                .map(|lane| count_calls(&lane.body, target))
                .sum(),
            AuthoredDeclaration::OperationCall(call) => {
                usize::from(target.is_none_or(|target| call.child_program_ref == *target))
                    + count_failure_calls(&call.failure_directive, target)
            }
        })
        .sum()
}

fn count_failure_calls(
    directive: &AuthoredFailureDirective,
    target: Option<&mfm_ids::ContentRef>,
) -> usize {
    match directive {
        AuthoredFailureDirective::Custom { arms, .. } => {
            arms.iter().map(|arm| count_calls(&arm.body, target)).sum()
        }
        AuthoredFailureDirective::NoFailure | AuthoredFailureDirective::Default => 0,
    }
}

fn contains_inline_lexical_slot(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => values.iter().any(contains_inline_lexical_slot),
        serde_json::Value::Object(object) => {
            (object.contains_key("lexical_path")
                && object.contains_key("contract_ref")
                && object.contains_key("producer"))
                || object.values().any(contains_inline_lexical_slot)
        }
        _ => false,
    }
}

fn count_direct_candidate_skip_arms(block: &AuthoredBlock) -> usize {
    block
        .declarations
        .iter()
        .map(|declaration| match declaration {
            AuthoredDeclaration::Match(binding) => binding
                .arms
                .iter()
                .map(|arm| {
                    let is_candidate_skip = arm.label.as_str().starts_with("candidate-skip-");
                    if is_candidate_skip {
                        assert!(arm.body.declarations.is_empty());
                        assert!(matches!(arm.body.tail, BlockTail::Normal(_)));
                    }
                    usize::from(is_candidate_skip) + count_direct_candidate_skip_arms(&arm.body)
                })
                .sum(),
            AuthoredDeclaration::FanOut(group) => group
                .lanes
                .iter()
                .map(|lane| count_direct_candidate_skip_arms(&lane.body))
                .sum(),
            AuthoredDeclaration::State(state) => {
                count_direct_failure_skip_arms(&state.failure_directive)
            }
            AuthoredDeclaration::OperationCall(call) => {
                count_direct_failure_skip_arms(&call.failure_directive)
            }
        })
        .sum()
}

fn count_direct_failure_skip_arms(directive: &AuthoredFailureDirective) -> usize {
    match directive {
        AuthoredFailureDirective::Custom { arms, .. } => arms
            .iter()
            .map(|arm| count_direct_candidate_skip_arms(&arm.body))
            .sum(),
        AuthoredFailureDirective::NoFailure | AuthoredFailureDirective::Default => 0,
    }
}
